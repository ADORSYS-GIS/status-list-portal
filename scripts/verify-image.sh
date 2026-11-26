#!/usr/bin/env bash
# Verifies that the release workflow built and signed a published image of the portal: the
# cosign signature and the build provenance attestation, both issued to the workflow run of
# the given release tag. The release workflow runs it before tagging the image; run it before
# deploying a digest. Needs cosign, and gh signed in to GitHub.
#
#   bash scripts/verify-image.sh ghcr.io/adorsys-gis/status-list-portal@sha256:... v0.1.0
set -euo pipefail

readonly OIDC_ISSUER=https://token.actions.githubusercontent.com
readonly PROVENANCE=https://slsa.dev/provenance/v1
readonly WORKFLOW=.github/workflows/release.yml

usage="usage: verify-image.sh <image>@sha256:<digest> <release tag>"
image=${1:?$usage}
tag=${2:?$usage}
# Set in GitHub Actions, so that a fork's releases verify against the fork.
repository=${GITHUB_REPOSITORY:-ADORSYS-GIS/status-list-portal}

# A tag can move between this check and the deployment; a digest cannot.
if [[ ! "$image" =~ ^[^[:space:]@]+@sha256:([a-f0-9]{64})$ ]]; then
    echo "'${image}' does not name a digest: verify <image>@sha256:<digest>, the one you deploy."
    exit 1
fi
digest=${BASH_REMATCH[1]}
if [[ ! "$tag" =~ ^v[0-9A-Za-z.-]+$ ]]; then
    echo "'${tag}' is not a release tag, such as v0.1.0."
    exit 1
fi

# Exactly the release workflow at this tag: the repository alone would also accept a
# signature from any other workflow or ref of it.
identity="https://github.com/${repository}/${WORKFLOW}@refs/tags/${tag}"

echo "Verifying the signature of ${image}"
cosign verify --certificate-identity "$identity" --certificate-oidc-issuer "$OIDC_ISSUER" \
    "$image" > /dev/null

echo "Verifying its build provenance"
# One line per verified subject: its digest, then the commit the run built, which the
# certificate takes from GitHub's token, not from the workflow.
# shellcheck disable=SC2016 # $commit is jq's.
attested=$(gh attestation verify "oci://${image}" --repo "$repository" \
    --cert-identity "$identity" --cert-oidc-issuer "$OIDC_ISSUER" \
    --source-ref "refs/tags/${tag}" --predicate-type "$PROVENANCE" \
    --deny-self-hosted-runners --format json \
    --jq '.[].verificationResult
        | .signature.certificate.sourceRepositoryDigest as $commit
        | .statement.subject[].digest.sha256 + " " + $commit')
# Some versions of gh exit 0 when they find no attestation, so one must name the digest.
commit=$(awk -v digest="$digest" '$1 == digest { print $2; exit }' <<< "$attested")
if [[ ! "$commit" =~ ^[0-9a-f]{40}$ ]]; then
    echo "No verified provenance names ${digest} and the commit it was built from."
    exit 1
fi

# The workflow checks this too, but a tag on another branch runs that branch's workflow,
# which may not.
echo "Checking that it was built from main"
status=$(gh api "repos/${repository}/compare/${commit}...main" --jq .status)
if [ "$status" != identical ] && [ "$status" != ahead ]; then
    echo "${image} was built from ${commit}, which is not on main."
    exit 1
fi

echo "${image} was built from ${commit} on main and signed by ${identity}"
