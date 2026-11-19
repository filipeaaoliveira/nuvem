---
title: Install
description: What nuvem needs, and how to install it with Homebrew, from a release archive, or from source.
sidebar:
  order: 1
---

Install nuvem with Homebrew, from a release archive, or from source.

## Requirements

- The [Google Cloud CLI](https://cloud.google.com/sdk/docs/install) (`gcloud`),
  installed and signed in with `gcloud auth login`. nuvem reads your gcloud
  configurations and asks `gcloud` for an access token, so it stores no
  credentials of its own.
- A terminal at least 60 columns wide and 12 rows tall.
- Linux or macOS. Process control uses POSIX APIs, so Windows works only
  inside WSL.
- Outbound HTTPS to `*.googleapis.com`.

Some features need one more program on your `PATH`. nuvem runs without these
programs. If one is missing, nuvem tells you when you use the feature that
needs it:

| Program | Used for | Flag to point at it |
| --- | --- | --- |
| `bq` | Browsing BigQuery datasets. It ships with the Google Cloud CLI. | `--bq-binary` |
| `k9s` | Opening a GKE cluster in k9s | `--k9s-binary` |
| `kubectl` | Checking whether a GKE kube context already exists before k9s opens | `--kubectl-binary` |
| `cloud-sql-proxy` | Cloud SQL tunnels | `--cloud-sql-proxy-binary` |
| `wl-copy`, `xclip` or `xsel` | Copying to the clipboard on Linux. macOS uses `pbcopy`. | none |

## Install with Homebrew

On macOS or Linux with [Homebrew](https://brew.sh):

```bash
brew install filipeaaoliveira/tap/nuvem
```

To upgrade later, run `brew upgrade nuvem`. Homebrew does not install the
Google Cloud CLI for you. If you do not have it, run
`brew install --cask gcloud-cli` and then `gcloud auth login`.

## Download a release

Each [GitHub release](https://github.com/filipeaaoliveira/nuvem/releases)
carries a `nuvem-<target>.tar.gz` archive and a `.sha256` file beside it.

| Platform | Target |
| --- | --- |
| Linux, x86_64 | `x86_64-unknown-linux-gnu` |
| Linux, arm64 | `aarch64-unknown-linux-gnu` |
| macOS, Apple silicon | `aarch64-apple-darwin` |
| macOS, Intel | `x86_64-apple-darwin` |

CI builds every archive on a runner of its own platform, runs the test suite
there, and smoke-tests the binary before packaging it.

1. Download the archive for your platform and its `.sha256` file.
2. Verify the archive against its checksum:

   ```bash
   shasum -a 256 -c nuvem-<target>.tar.gz.sha256
   ```

3. Unpack it and put the binary on your `PATH`:

   ```bash
   tar xzf nuvem-<target>.tar.gz && install nuvem-<target>/nuvem ~/.local/bin/
   ```

The Linux binaries need glibc 2.28 or newer. Ubuntu 20.04, Debian 10, RHEL 8
and later releases have it. To see your version, run `ldd --version`. With an
older glibc, nuvem fails at startup with `GLIBC_2.xx not found`.

Every archive also carries a signed build-provenance attestation. It records
the workflow, commit and run that built the archive. The checksum shows that
the download is intact, and the attestation shows where it came from. To
verify the attestation with the GitHub CLI, run:

```bash
gh attestation verify nuvem-x86_64-unknown-linux-gnu.tar.gz --repo filipeaaoliveira/nuvem
```

## Build from source

You need a recent stable Rust toolchain, a C toolchain and `cmake` on your
`PATH`. The C toolchain and `cmake` are for the TLS library that the Google
Cloud client libraries use, which compiles native code at build time. If you
use `rustup`, it installs the pinned Rust version for you.

```bash
git clone https://github.com/filipeaaoliveira/nuvem
cd nuvem
cargo install --path . --locked
```

## Check the install

```bash
nuvem --version
nuvem info
```

`nuvem info` prints the config, plugin, log and cache paths nuvem uses, and
any warning about your config file. It does not start the interface. Next,
read [First run](first-run.md).
