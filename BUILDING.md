# Native package builds

The PR workflow builds Edit independently of the OS workspace using the pinned
DevKit setup Action and published shared.8 SDK. It runs host unit tests, builds
the native mochiOS executable, and uses the public `build-mpkg` Action to create
an unsigned package. Native programs are inspected, not executed on Linux.

Download `edit-unsigned-mpkg` from the successful Actions run. It contains
`Edit.mpkg`, `SHA256SUMS`, `build-info.json`, and `resolved.Cargo.lock`. Review and
commit an updated dependency resolution when changing SDK/source pins. Runner
paths and the local SDK description are not distributed. The normal source
manifest still supports built-in/preview staging; the package build derives
digest-bound ordinary `/applications/Edit.app` installation metadata without
built-in provenance, preserving declared resources, document roles and capabilities.

Changes go to `staging` and a PR targeting `main`. PRs never sign or release.
After merging, the default-branch workflow builds again and, once releases have
been enabled, waits for approval in the `app-release` Environment. That
environment is restricted to the default branch and reviewed by
`mochiOS/application-maintainers`.

Publication is initially disabled. Before setting the repository variable
`APP_RELEASE_ENABLED=true`, configure these Environment values:

* Secret `MSIGN_CERTIFICATE_B64`: base64 of the issued app Developer Certificate file.
* Secret `MSIGN_PRIVATE_KEY_B64`: base64 of its corresponding private-key file.
* Variable `MSIGN_ROOT_PUBLIC_KEY_B64`: base64 of the trusted issuer public-key file.

Do not commit key files or use development fixture identities. The approval job
does not check out or run application code. It checks the build's source commit,
SDK Action revision, actual package ID, and unsigned checksum, signs with msign,
checks that the configured Team approval and branch restrictions are still present,
then verifies the signature/certificate/payload before publishing. It creates a
prerelease named `v<version>-r<revision>` and never replaces existing tags or assets.
Increase `package.revision` before another release of the same version. An
approval cannot override a missing, invalid, expired or insufficient signing identity.
