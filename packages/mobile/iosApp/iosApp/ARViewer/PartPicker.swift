import simd

/// Ray maths for tapping parts. Kept free of RealityKit so it can be unit tested.
enum PartPicker {
    /// How far along the ray it first enters the box (in units of `direction`), or nil if it misses,
    /// the box is behind, or the ray starts inside it.
    static func hitDistance(origin: SIMD3<Float>, direction: SIMD3<Float>, boxMin: SIMD3<Float>, boxMax: SIMD3<Float>) -> Float? {
        var near = -Float.infinity
        var far = Float.infinity
        for axis in 0..<3 {
            let o = origin[axis], d = direction[axis]
            if abs(d) < 1e-9 {
                if o < boxMin[axis] || o > boxMax[axis] { return nil }
                continue
            }
            var t0 = (boxMin[axis] - o) / d
            var t1 = (boxMax[axis] - o) / d
            if t0 > t1 { swap(&t0, &t1) }
            near = max(near, t0)
            far = min(far, t1)
            if near > far { return nil }
        }
        return near >= 0 ? near : nil
    }

    /// The ray moved into a space with the given world-from-local transform.
    /// Distances along it keep their meaning, because affine maps preserve the ray parameter.
    static func ray(origin: SIMD3<Float>, direction: SIMD3<Float>, into worldFromLocal: simd_float4x4) -> (origin: SIMD3<Float>, direction: SIMD3<Float>) {
        let localFromWorld = worldFromLocal.inverse
        let o = localFromWorld * SIMD4(origin, 1)
        let d = localFromWorld * SIMD4(direction, 0)
        return (SIMD3(o.x, o.y, o.z), SIMD3(d.x, d.y, d.z))
    }
}

extension PartPicker {
    /// The face of a die-shaped box (in the die's own frame) a ray first
    /// enters through, or nil if it misses.
    static func entryFace(origin: SIMD3<Float>, direction: SIMD3<Float>, boxMin: SIMD3<Float>, boxMax: SIMD3<Float>) -> DieFace? {
        guard let t = hitDistance(origin: origin, direction: direction, boxMin: boxMin, boxMax: boxMax) else { return nil }
        let hit = origin + direction * t
        let size = boxMax - boxMin
        // The face the hit point lies on: the one it's closest to, relative to the box.
        let candidates: [(DieFace, Float)] = [
            (.px, abs(hit.x - boxMax.x) / size.x), (.nx, abs(hit.x - boxMin.x) / size.x),
            (.py, abs(hit.y - boxMax.y) / size.y), (.ny, abs(hit.y - boxMin.y) / size.y),
            (.pz, abs(hit.z - boxMax.z) / size.z), (.nz, abs(hit.z - boxMin.z) / size.z),
        ]
        return candidates.min { $0.1 < $1.1 }?.0
    }
}
