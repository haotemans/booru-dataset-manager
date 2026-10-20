# Release and Windows packaging policy

Windows is the primary target; Linux CI is useful but does not prove Windows correctness. Before a release, record the exact Rust toolchain, target triple, dependency lockfile, build command, artifact checksum and test evidence. Use reproducible version tags and release notes that separate implemented features from known limitations.

## Release gates

- CI format, tests and clippy pass on the release commit.
- Windows build and launch smoke test pass on a documented supported Windows version.
- Test paths containing Unicode, spaces and long directory names.
- Verify sidecar atomic replacement and quarantine behavior on NTFS.
- Verify credential storage, proxy/network error handling and cancellation.
- Verify database migration and backup/restore against a clean test profile.
- Confirm model weights are separately supplied, license reviewed and runtime tested.
- Confirm installer/update/uninstall behavior and that user library files are not removed by uninstall.
- Publish SHA-256 for release artifacts and document upgrade/rollback steps.

Do not claim code signing, installer security, Windows compatibility, provider support, DINOv3 inference or ModelScope restore unless tested in the relevant environment.
