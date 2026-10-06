import ARKit
import Vision
import simd

/// Finds one hand in the camera image and places its thumb and index
/// fingertips in the world, for picking the die up with a pinch.
///
/// Vision gives the fingertips in the image; LiDAR's scene depth (so this
/// needs a device with LiDAR) says how far away they are, and the camera's
/// intrinsics turn that into a point in the world. One frame is worked on at
/// a time, off the main thread; frames that arrive meanwhile are skipped.
final class HandTracker: @unchecked Sendable {
    /// One hand, seen in one frame.
    struct Reading: Sendable {
        /// Thumb and index fingertips, in world space.
        let thumb: SIMD3<Float>
        let index: SIMD3<Float>
        /// Halfway between them: where a pinch holds things.
        var pinch: SIMD3<Float> { (thumb + index) / 2 }
        var gap: Float { simd_distance(thumb, index) }
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
              let hand = request.results?.first,
              let thumb = try? hand.recognizedPoint(.thumbTip),
              let index = try? hand.recognizedPoint(.indexTip),
              thumb.confidence > DiePhysics.handMinConfidence, index.confidence > DiePhysics.handMinConfidence
        else { return nil }

        // Vision's points run from the bottom left; the image's from the top left.
        let thumbUV = SIMD2(Float(thumb.location.x), 1 - Float(thumb.location.y))
        let indexUV = SIMD2(Float(index.location.x), 1 - Float(index.location.y))
        guard let thumbDepth = Self.depth(at: thumbUV, in: depth),
              let indexDepth = Self.depth(at: indexUV, in: depth)
        else { return nil }
        return Reading(thumb: Self.unproject(thumbUV, depth: thumbDepth, camera: camera),
                       index: Self.unproject(indexUV, depth: indexDepth, camera: camera))
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
        let pixel = uv * camera.resolution
        let k = camera.intrinsics
        let fx = k[0][0], fy = k[1][1], cx = k[2][0], cy = k[2][1]
        // ARKit's camera space: x right, y up, looking down −z; the image's y runs down.
        let local = SIMD4<Float>((pixel.x - cx) / fx * depth, -(pixel.y - cy) / fy * depth, -depth, 1)
        let world = camera.transform * local
        return SIMD3(world.x, world.y, world.z)
    }
}
