## Summary

<!-- What does this change, and why? Link the issue or spec section (e.g. SIM_SPEC H9) if there is one. -->

## Packages touched

- [ ] `firmware`
- [ ] `simulator`
- [ ] `server`
- [ ] `mobile`
- [ ] `shared`
- [ ] `docs` / tooling

## How it was tested

<!-- Commands you ran and what you saw. For simulator or UI changes, say what you tried in the browser or attach a screenshot. -->

## Checklist

- [ ] `cargo fmt --all --check` and clippy are clean (firmware, simulator, shared)
- [ ] Tests pass for the packages touched (`cargo test`, `./gradlew :shared:testDebugUnitTest` for mobile)
- [ ] Docs updated if behaviour, endpoints or the wire/roll format changed (`docs/`)
- [ ] Screen snapshots regenerated on purpose, if rendering changed (`UPDATE_SNAPSHOTS=1 cargo test -p smokebomb-core --test snapshots`)
