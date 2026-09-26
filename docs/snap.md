# Publishing rask as a snap

The recipe is [`snap/snapcraft.yaml`](../snap/snapcraft.yaml). CI builds it
for amd64 and arm64 ([`.github/workflows/snap.yml`](../.github/workflows/snap.yml)).

## Which systems it supports

- **Every Ubuntu LTS release** (20.04, 22.04, 24.04, 26.04), and any other
  distribution with snapd. The snap contains one statically linked binary
  (musl libc, with PCRE2 and mimalloc built in), so it doesn't depend on the
  host's libraries or the base snap's. `base: core24` only sets the build
  environment.
- **amd64 and arm64.** Each is built natively, on `ubuntu-24.04` and
  `ubuntu-24.04-arm` GitHub runners.

## Confinement: classic

A search tool has to read whatever it's pointed at, including dotfiles
such as `~/.ackrc`, `/etc/ackrc`, and system trees such as `/usr/include`.
Strict confinement can't do that: the `home` interface excludes dotfiles,
and nothing outside home and removable media is allowed. So the snap is
**classic**, as the ripgrep snap is. Classic confinement needs a one-time
approval from the Snap Store reviewers (step 3 below).

## One-time setup (needs your Snap Store account)

1. **Install snapcraft** (already done on the dev VM):

       sudo snap install snapcraft --classic

2. **Log in and register the name:**

       snapcraft login
       snapcraft register rask

   `rask` isn't published by anyone, but it may be registered or reserved.
   If `register` refuses it, pick another name, change `name:` in
   `snap/snapcraft.yaml`, and ask again.

3. **Request classic confinement.** Post in the *store-requests* category
   of https://forum.snapcraft.io, titled "Classic confinement request:
   rask". Explain that rask is a grep-like search tool (a reimplementation
   of ack) that must read arbitrary user-chosen files and directories,
   including dotfiles and system paths. Mention ripgrep as a precedent.
   Uploads are held until the request is approved.

4. **Give CI upload rights.** Create a store credential limited to this
   snap, and save it as the GitHub secret `SNAPCRAFT_STORE_CREDENTIALS`
   (Settings → Secrets and variables → Actions):

       snapcraft export-login --snaps=rask \
         --acls package_access,package_push,package_update,package_release -

5. **Optional: the `ack` command.** Snap commands are named after the snap.
   To make `ack` run rask, request an auto-alias (`rask` → `ack`) in
   *store-requests*. Until then, users can add it themselves:

       sudo snap alias rask ack

## Releasing

- **Push a tag** such as `v0.1.0`. CI builds both architectures,
  smoke-tests each snap, and uploads it to the `edge` channel. Check that
  the version in `Cargo.toml` matches the tag: the snap takes its version
  from `Cargo.toml`.
- **Promote** once it's tested, from the web dashboard or with:

      snapcraft status rask
      snapcraft release rask <revision> beta,candidate,stable

Every push to `main` also builds both snaps, and they're kept as workflow
artifacts (`rask-snap-amd64`, `rask-snap-arm64`) for testing.

## Building locally

- **In LXD (the default).** Needs about 1.5 GB of free disk:

      snapcraft pack

- **On an Ubuntu 24.04 host, without a container.** Uses the host's `stable`
  Rust (through the `rustup` snap) and never updates it:

      sudo snap install rustup --classic
      sudo apt install musl-tools
      snapcraft pack --destructive-mode

  On the dev VM, build from a copy of the repo on the VM's own disk. In
  the VirtualBox shared folder the linker writes corrupt binaries.

- **arm64 from an amd64 machine:** `snapcraft remote-build` builds on
  Launchpad (needs a Launchpad account), or use CI.

Install a local build to test it:

    sudo snap install --dangerous --classic rask_0.1.0_amd64.snap
