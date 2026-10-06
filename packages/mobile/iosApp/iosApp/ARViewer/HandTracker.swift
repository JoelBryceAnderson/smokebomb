import ARKit
import Vision
import simd

/// Finds one hand in the camera image and places its thumb and index
/// fingertips in the world, for picking the die up with a pinch, and which
/// way the palm faces, for turning it.
///
/// Vision gives the fingertips in the image; LiDAR's scene depth (so this
/// needs a device with LiDAR) says how far away they are, and the camera's
/// intrinsics turn that into a point in the world. One frame is worked on at
/// a time, off the main thread; frames that arrive meanwhile are skipped.
final class HandTracker: @unchecked Sendable {
    /// One hand, seen in one frame. With the thumb or index tip hidden (as
    /// they often are, pinched), only that a hand is there is known.
    struct Reading: Sendable {
        /// Halfway between the thumb and index tips, in world space: where a
        /// pinch holds things.
        let pinch: SIMD3<Float>?
        /// How far apart the tips are, in metres.
        let gap: Float?
        /// Which way the hand points and the palm faces, in world space; nil
        /// with the wrist or knuckles hidden.
        let palm: Palm?
    }

    /// The hand's own axes, both unit length: along the hand (wrist to
    /// middle knuckle) and out of the palm (its sign follows the hand's
    /// chirality, which a grab doesn't change).
    struct Palm: Sendable {
        let forward: SIMD3<Float>
        let normal: SIMD3<Float>
    }

    /// Whether this device can: it needs LiDAR's scene depth.
    static var isSupported: Bool {
        ARWorldTrackingConfiguration.supportsFrameSemantics(.sceneDepth)
    }

    private let queue = DispatchQueue(label: "smokebomb.hand-tracker", qos: .userInteractive)
    private let request: VNDetectHumanHandPoseRequest = {
        let r = VNDetectHumanHandPoseRequest()
        r.maximumHandCount = 1
        return r
    }()
    /// Main thread only: a frame is being worked on.
    private var busy = false

    /// Looks for a hand in `frame`, unless the last frame is still being
    /// worked on. `done` gets the hand (nil if none was found) on the main actor.
    @MainActor
    func submit(_ frame: ARFrame, done: @escaping @MainActor (Reading?) -> Void) {
        guard !busy, let depth = (frame.smoothedSceneDepth ?? frame.sceneDepth)?.depthMap else { return }
        busy = true
        let image = frame.capturedImage
        let camera = Camera(frame.camera)
        queue.async { [self] in
            let reading = self.find(in: image, depth: depth, camera: camera)
            Task { @MainActor in
                self.busy = false
                done(reading)
            }
        }
    }

    /// What's needed of the ARCamera, copied so the frame can go.
    private struct Camera {
        let intrinsics: simd_float3x3
        let resolution: SIMD2<Float>
        let transform: simd_float4x4

        init(_ c: ARCamera) {
            intrinsics = c.intrinsics
            resolution = SIMD2(Float(c.imageResolution.width), Float(c.imageResolution.height))
            transform = c.transform
        }
    }

    private func find(in image: CVPixelBuffer, depth: CVPixelBuffer, camera: Camera) -> Reading? {
        // The captured image is in the sensor's own orientation, as are the
        // intrinsics and the depth map, so no rotation is needed anywhere.
        let handler = VNImageRequestHandler(cvPixelBuffer: image, orientation: .up)
        guard (try? handler.perform([request])) != nil,
              let hand = request.results?.first
        else { return nil }
        let palm = Self.palm(of: hand, depth: depth, camera: camera)
        guard let thumb = try? hand.recognizedPoint(.thumbTip),
              let index = try? hand.recognizedPoint(.indexTip),
              thumb.confidence > DiePhysics.handMinConfidence, index.confidence > DiePhysics.handMinConfidence
        else { return Reading(pinch: nil, gap: nil, palm: palm) }

        // Vision's points run from the bottom left; the image's from the top left.
        let thumbUV = SIMD2(Float(thumb.location.x), 1 - Float(thumb.location.y))
        let indexUV = SIMD2(Float(index.location.x), 1 - Float(index.location.y))
        // Both tips at the nearer one's depth: two separate depths are noisy
        // enough apart to open a pinch that's still closed.
        let depths = [Self.depth(at: thumbUV, in: depth), Self.depth(at: indexUV, in: depth)].compactMap { $0 }
        guard let near = depths.min() else { return Reading(pinch: nil, gap: nil, palm: palm) }
        let pixels = simd_distance(thumbUV * camera.resolution, indexUV * camera.resolution)
        let gap = pixels * near / camera.intrinsics[0][0]
        return Reading(pinch: Self.unproject((thumbUV + indexUV) / 2, depth: near, camera: camera), gap: gap, palm: palm)
    }

    /// The palm's axes, from the wrist and the index, middle and little
    /// knuckles: broad, flat and seldom hidden by a pinch, so their depths
    /// hold up better than the fingertips'.
    ///
    /// Scene depth is smoothed across a hand, so a palm tipped toward or
    /// away from the camera reads flatter than it is: the joints' depths
    /// are spread about their mean by `handDepthGain` to make up for it.
    /// Turns across the view come from the image alone and need no help.
    private static func palm(of hand: VNHumanHandPoseObservation, depth map: CVPixelBuffer, camera: Camera) -> Palm? {
        func point(_ joint: VNHumanHandPoseObservation.JointName) -> (uv: SIMD2<Float>, depth: Float)? {
            guard let p = try? hand.recognizedPoint(joint), p.confidence > DiePhysics.handPalmMinConfidence else { return nil }
            let uv = SIMD2(Float(p.location.x), 1 - Float(p.location.y))
            guard let d = depth(at: uv, in: map) else { return nil }
            return (uv, d)
        }
        guard let wrist = point(.wrist), let index = point(.indexMCP),
              let middle = point(.middleMCP), let little = point(.littleMCP)
        else { return nil }
        let joints = [wrist, index, middle, little]
        let mean = joints.reduce(0) { $0 + $1.depth } / Float(joints.count)
        let placed = joints.map { j in
            cameraPoint(j.uv, depth: mean + (j.depth - mean) * DiePhysics.handDepthGain, camera: camera)
        }
        let forward = placed[2] - placed[0]
        let normal = simd_cross(forward, placed[3] - placed[1])
        // Too foreshortened to tell which way it's turned.
        guard simd_length(forward) > 0.03, simd_length(normal) > 1e-4 else { return nil }
        let toWorld = simd_float3x3(columns: (
            SIMD3(camera.transform.columns.0.x, camera.transform.columns.0.y, camera.transform.columns.0.z),
            SIMD3(camera.transform.columns.1.x, camera.transform.columns.1.y, camera.transform.columns.1.z),
            SIMD3(camera.transform.columns.2.x, camera.transform.columns.2.y, camera.transform.columns.2.z)))
        return Palm(forward: simd_normalize(toWorld * forward), normal: simd_normalize(toWorld * normal))
    }

    /// The depth at a point of the image (0…1, from the top left), in metres.
    /// A fingertip is small against the depth map, so this takes a near
    /// value from the few pixels around it: the finger, not what's behind it.
    private static func depth(at uv: SIMD2<Float>, in map: CVPixelBuffer) -> Float? {
        CVPixelBufferLockBaseAddress(map, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(map, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(map) else { return nil }
        let width = CVPixelBufferGetWidth(map), height = CVPixelBufferGetHeight(map)
        let rowBytes = CVPixelBufferGetBytesPerRow(map)
        let cx = Int(uv.x * Float(width)), cy = Int(uv.y * Float(height))
        let r = DiePhysics.handDepthWindow
        var samples: [Float] = []
        for y in max(cy - r, 0)...min(cy + r, height - 1) {
            let row = (base + y * rowBytes).assumingMemoryBound(to: Float32.self)
            for x in max(cx - r, 0)...min(cx + r, width - 1) where row[x].isFinite && row[x] > 0 {
                samples.append(row[x])
            }
        }
        guard !samples.isEmpty else { return nil }
        samples.sort()
        return samples[samples.count / 4]
    }

    /// A point of the image (0…1, from the top left) at `depth` metres, in world space.
    private static func unproject(_ uv: SIMD2<Float>, depth: Float, camera: Camera) -> SIMD3<Float> {
        let local = cameraPoint(uv, depth: depth, camera: camera)
        let world = camera.transform * SIMD4(local, 1)
        return SIMD3(world.x, world.y, world.z)
    }

    /// The same, in the camera's space: x right, y up, looking down −z.
    private static func cameraPoint(_ uv: SIMD2<Float>, depth: Float, camera: Camera) -> SIMD3<Float> {
        let pixel = uv * camera.resolution
        let k = camera.intrinsics
        let fx = k[0][0], fy = k[1][1], cx = k[2][0], cy = k[2][1]
        // The image's y runs down.
        return SIMD3((pixel.x - cx) / fx * depth, -(pixel.y - cy) / fy * depth, -depth)
    }
}
