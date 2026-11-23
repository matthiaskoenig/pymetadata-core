# Release

How a release of `pymetadata-core` is made. A tag makes all of it: the wheels and the sdist on PyPI, the package on npm and the release of this repository, whose body is the release notes of that version. None of them is made by hand, and nothing goes to crates.io.

## The release notes

`release-notes/x.y.z.md` is the body of the release of `vx.y.z`. The job `publish` of `.github/workflows/release.yml` reads it before it uploads anything, so a tag whose notes were forgotten fails without spending the version number: a filename of PyPI cannot be taken back. Write what a reader needs: what changed, what it is worth where there are numbers (from `BENCHMARKS.md`, never from memory), what a caller has to do differently and the known limitations. The numbers are measurements of one machine, so say what was compared.

## The steps

1. branch off `develop`: `git switch -c release/x.y.z develop`
2. write `release-notes/x.y.z.md`
3. set `version` of `[workspace.package]` in `Cargo.toml`, set the same version in the `version` of every path dependency on a crate of this workspace (`pymetadata-core` in `crates/python/Cargo.toml` and in `crates/wasm/Cargo.toml`), set the same version in `js/package.json`, and refresh `Cargo.lock` (`cargo metadata --format-version 1 > /dev/null`). The pins are what a consumer of the published crates resolves against, so `cargo metadata` fails until they match; `js/prepack.mjs` refuses to pack a package whose version is not the version of the crates. The python version is read from the crate, so there is nothing else to set. `uv.lock` records no version of `pymetadata-core`, since it is dynamic.
4. make sure everything passes, in the order of [Checks](../README.md#checks)
5. push the branch, open the pull request against `develop` and merge it once the checks are green. A pull request which changes `release.yml` also builds the wheels and the sdist, and publishes nothing.
6. tag the merged commit on `develop` and push the tag. Check the version before pushing: the tag publishes to PyPI, which cannot be taken back.

    ```bash
    git switch develop
    git pull
    git tag vx.y.z
    git push origin vx.y.z
    ```

7. check the published artifacts rather than the workflow, from a directory which is not this checkout:

    ```bash
    uv venv --python 3.13 /tmp/release-check && uv pip install --python /tmp/release-check/bin/python --refresh --upgrade "pymetadata-core==x.y.z"
    /tmp/release-check/bin/python -c "import pymetadata_core; print(pymetadata_core.__version__)"
    cd /tmp && npm init -y && npm install pymetadata-core@x.y.z
    ```

    The example of `js/README.md` runs as it stands.

The tag builds the five wheels and the sdist, uploads them with trusted publishing and PEP 740 attestations, publishes `js/` with trusted publishing and a provenance attestation, and creates the release `pymetadata-core x.y.z` from `release-notes/x.y.z.md`. The release is made only once both uploads succeeded. Every publishing step is idempotent, so a failed run is re-run from the failed job: the PyPI upload uses `skip-existing`, the npm job skips a version npm already has, and the release job stops when the release exists. What a re-run cannot fix is a mistake in the tagged commit itself (the tag is immutable, see below); that needs the next version. After a partial upload, a re-run skips the files already on PyPI and uploads the rest from a fresh build, so the published set can hold files from two builds. The tag fixes the commit, so this cannot publish different code, but the artifacts may not be byte-identical siblings. The npm path is exercised without publishing by the job `npm_dry_run`, on every run of the workflow.

The `tags` ruleset forbids deleting or moving a tag. A tag which failed before the upload therefore stays; lift the ruleset for the correction, or release the next patch version.

## One-time setup

Done once, by the maintainer, when the repository is created.

1. the repository policies: `.github/rulesets/apply.sh` (needs `gh auth login` as an admin). It applies the merge settings and the rulesets `develop` (pull request only, with the required checks `checks`, `wasm`, `js`, `python`, `ruff` and `ty`, the jobs of `ci.yml`) and `tags`. A renamed or added job of `ci.yml` is a change of `develop.json` in the same pull request, or merges block on a check which never runs.
2. PyPI: a pending publisher for the project `pymetadata-core`, repository `matthiaskoenig/pymetadata-core`, workflow `release.yml`, environment `pypi`; and the environment `pypi` in the settings of the repository.
3. npm: the package has to exist before a trusted publisher can be configured on it, so the first publish is manual, before the tag of the first release (the job `github_release` needs the job `npm`). From the merged `develop`:

    ```bash
    node js/build.mjs
    cd js
    npm login
    node prepack.mjs
    npm publish --access public
    ```

    Then set the trusted publisher on npmjs.com (repository `matthiaskoenig/pymetadata-core`, workflow `release.yml`). The job finds the version on npm and skips its publish with a notice.
