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
