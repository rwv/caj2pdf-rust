# Docker CLI

The multiarch image targets Linux amd64, arm64, ARMv5/v6/v7, 386, LoongArch64,
RISC-V64 and PowerPC64 little/big endian. The exact OCI platform mapping is
`container-platforms.json`. It contains the tested static
musl executable and license notices on `scratch`: no shell, package manager,
network service, GUI or vendor converter. Default user is 65532:65532. Docker
provides convenient deployment; it does not reduce conversion memory or remove
the need for temporary disk storage.

## Convert a mounted document

Use the version tag from the release notes (or its immutable registry digest):

```sh
docker run --rm --read-only \
  --user "$(id -u):$(id -g)" \
  --tmpfs /tmp:rw,noexec,nosuid,mode=1777,size=1g \
  --mount "type=bind,src=$PWD,dst=/data" \
  ghcr.io/rwv/caj2pdf-rust:v0.3.0 input.caj -o output.pdf
```

The mounted directory must be writable by the selected user. Choose temporary
storage large enough for forward-only input and bounded HN/C8 scratch. A tmpfs
counts against container/host memory; for large inputs, bind-mount a private disk
directory at `/tmp` instead. Container termination or a second interrupt can
prevent cooperative cleanup; remove abandoned host scratch files when the
container no longer owns them.

For stdin/stdout (binary output stays on stdout):

```sh
docker run --rm -i --read-only --tmpfs /tmp:rw,noexec,nosuid,mode=1777,size=1g \
  ghcr.io/rwv/caj2pdf-rust:v0.3.0 - < input.caj > output.pdf
```

C8/HN-B needs `--no-bookmarks`. All native format limits and resource limits
still apply. No exposed port or background service is needed.

## Verification and offline distribution

CI exports one multiarch OCI archive, imports each architecture from that exact
archive and runs conversion/pipe tests with a read-only root filesystem. The
same archive is uploaded as a release asset and copied to GHCR without rebuilding.
The release records SHA256SUMS and the registry manifest digest. Registry access
and publication are verified separately from a successful local image build.

The image contains project-owned MIT code and selected dependency MIT notices.
The Rust toolchain and statically linked system runtime retain their respective
upstream notices/terms; the project license does not relicense those components.
