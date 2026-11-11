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

Publication is initially disabled. Initialize the identity on Actions before
setting the repository variable `APP_RELEASE_ENABLED=true`:

1. Merge the reviewed workflow changes and wait for the main-branch package run
   to succeed. Copy its run ID from the Actions URL. PR artifacts are refused.
2. Create a short-lived fine-grained PAT with only Edit.app selected and
   Environments read/write. Save it in the `app-release` Environment as
   `SIGNING_SETUP_TOKEN`. Do not paste the token into workflow inputs or logs.
3. Run **Initialize Edit signing** on `main`, supplying the successful build run
   ID. The existing `application-maintainers` protection requires approval.
4. Open the running **Authorize Kome and save the verified signing identity**
   step. Follow its public device-login URL in a browser and authorize the
   official mochiOS Developer. No account tokens are copied into GitHub inputs.
5. After the workflow succeeds, revoke the temporary PAT in GitHub settings.
   The workflow deletes its `SIGNING_SETUP_TOKEN` Secret, but this alone does
   not revoke the PAT itself.

The workflow generates Edit's key on the ephemeral hosted runner, obtains an
exact-package Developer Certificate and verifies a signed copy of the selected
MPKG. It atomically saves `MSIGN_IDENTITY_B64` (certificate, private key and
issuer public key) as one Environment Secret. It never uploads credential files,
publishes a release, enables publication, or checks out application code.

An encrypted `MSIGN_SETUP_KEYPAIR_B64` recovery Secret is retained if issuance
fails, so the next approved attempt reuses the same key. On success it is
removed. Existing production credentials are never silently overwritten;
rotation requires a separate explicit procedure. Temporary CLI sessions are
logged out and runner credential files are cleaned. Neither Root nor
Intermediate private keys are needed by this workflow.

Do not commit key files or use development fixture identities. The approval job
does not check out or run application code. It checks the build's source commit,
SDK Action revision, actual package ID, and unsigned checksum, signs with msign,
checks that the configured Team approval and branch restrictions are still present,
then verifies the signature/certificate/payload before publishing. It creates a
prerelease named `v<version>-r<revision>` and never replaces existing tags or assets.
Increase `package.revision` before another release of the same version. An
approval cannot override a missing, invalid, expired or insufficient signing identity.
