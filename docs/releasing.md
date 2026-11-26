# Releasing the portal

A release is a tag on `main`, such as `v0.2.0`, matching the version in `Cargo.toml` and with an entry in `CHANGELOG.md`. [release-plz](https://release-plz.dev) prepares both from the conventional commits and pushes the tag.

## Cutting a release

1. In the repository's Actions tab, run the **Release-plz** workflow on `main`. It opens a pull request, `chore: release v0.2.0`, that bumps the version in `Cargo.toml` and `Cargo.lock` and adds the release's entry to `CHANGELOG.md`. If that pull request is open already, it updates it with what was merged since.
2. Review and merge the pull request like any other. Reword the changelog entry on its branch if a line needs it.
3. Once it is merged, the Release-plz workflow tags the merge commit and starts the release workflow on the tag.

The version follows the commits since the last release. Before 1.0, a breaking change or a feature bumps the minor version, and only fixes bump the patch version, so that `0.2` only ever gains fixes; from 1.0, a breaking change bumps the major version. The changelog lists features, fixes, performance changes and breaking changes, and leaves out tests, documentation, CI and chores, as well as merge commits.

To release another version than the one proposed, such as a prerelease `0.2.0-rc.1`, change `version` in `Cargo.toml` on the pull request's branch, run `cargo check` to update `Cargo.lock`, and change the entry's heading in `CHANGELOG.md` to match. A prerelease is published as one: tagged `0.2.0-rc.1` only, leaving `0.2` on the last release.

Without release-plz, a pull request that changes the version and adds its changelog entry does the same, once merged, by pushing the tag by hand: `git tag v0.2.0 origin/main && git push origin v0.2.0`.

### Why the workflows are dispatched

GitHub starts no workflow for a pull request or tag that a workflow's `GITHUB_TOKEN` made, except one it dispatches. So the Release-plz workflow dispatches CI on the release pull request, and the release workflow on the tag. If the pull request's checks are missing, close and reopen it, which starts CI.

A tag ruleset that limits who can push `v*` tags would block the Release-plz workflow too, unless it used a GitHub App token that the ruleset lets through, as the Status List Server's does.

## The release workflow

The [release workflow](../.github/workflows/release.yml):

1. refuses a tag that differs from `Cargo.toml`'s version, tags a commit off `main`, has no `CHANGELOG.md` entry, or is released already;
2. runs CI on the tagged commit;
3. builds the image for `linux/amd64` and `linux/arm64` and pushes it by digest to `ghcr.io/adorsys-gis/status-list-portal`, with BuildKit's SBOM and provenance;
4. signs the digest with cosign, keyless, and attests its build provenance with GitHub's artifact attestations;
5. on an amd64 and an arm64 runner, pulls the digest without credentials, verifies its signature and provenance, and runs the CI image checks on it;
6. tags the digest `X.Y.Z` and `X.Y`;
7. creates the GitHub release, with the changelog entry and the digest in its notes.

Nothing is tagged until every check passes, so `X.Y.Z` always names a signed image that works.

The signature and attestation certificates name the release workflow at the tag, and Sigstore's public Rekor log keeps them for good.

### The first release

GHCR creates the package on the first push, private. The check jobs then fail, because they pull without credentials, as the cluster does. In the package's settings, under **Danger Zone**, change its visibility to public, then re-run the failed jobs. The organization must allow public packages.

### A failed release

If the workflow fails before it tags the image, nothing points to what it pushed. Fix the cause on `main`, then move the tag to the fix:

```sh
git push --delete origin v0.2.0
git fetch origin
git tag --force v0.2.0 origin/main
git push origin v0.2.0
```

Once the release exists, leave its tag alone and release a new version instead. To roll a deployment back, deploy the previous release's digest: every release keeps its own.

## Verifying an image

Before deploying a digest, check that this repository's release workflow built and signed it. With [cosign](https://docs.sigstore.dev/cosign/system_config/installation/) and [gh](https://cli.github.com) signed in to GitHub:

```sh
bash scripts/verify-image.sh ghcr.io/adorsys-gis/status-list-portal@sha256:<digest> v0.2.0
```

Without a checkout of the repository, these two commands check the signature and the provenance, and the script adds the checks after them:

```sh
identity=https://github.com/ADORSYS-GIS/status-list-portal/.github/workflows/release.yml@refs/tags/v0.2.0
image=ghcr.io/adorsys-gis/status-list-portal@sha256:<digest>
cosign verify --certificate-identity "$identity" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com "$image"
gh attestation verify "oci://$image" --repo ADORSYS-GIS/status-list-portal \
  --cert-identity "$identity" --source-ref refs/tags/v0.2.0 --deny-self-hosted-runners
```

Both pin the exact workflow and tag. A looser check, such as `--repo` alone, would also accept a signature from another workflow or branch of the repository. The script also checks that the verified provenance names the digest, since some versions of gh exit 0 without finding any, and that the commit its certificate names is on `main`: a tag on another branch runs that branch's copy of the workflow, which may skip the workflow's own check.

A failed release can leave a signed image that was never tagged. Only the digests in the releases' notes are releases.
