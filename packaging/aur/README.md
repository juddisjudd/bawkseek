# AUR package (bawkseek-bin)

The release workflow updates [bawkseek-bin](https://aur.archlinux.org/packages/bawkseek-bin) on the Arch User Repository after each tagged release. It repackages the release's `bawkseek-v<version>-linux-x64.tar.gz`.

It only runs once the `AUR_SSH_PRIVATE_KEY` secret exists. Without it, the step prints a notice and skips.

## One-time setup

1. Create an account at [aur.archlinux.org](https://aur.archlinux.org).
2. Make a key just for this, without a passphrase (CI cannot type one):

   ```sh
   ssh-keygen -t ed25519 -f aur -C "bawkseek AUR" -N ""
   ```

3. Add the contents of `aur.pub` to **My Account → SSH Public Key** on the AUR. The field takes one key per line, so keys for other packages can stay.
4. In this GitHub repository, open **Settings → Secrets and variables → Actions**. Add a secret named `AUR_SSH_PRIVATE_KEY` with the contents of `aur`.
5. Delete the local `aur` file, or keep it only in your password manager. If it ever leaks, make a new key and replace it on the AUR.

The next release creates the package on the AUR (the first push registers the name) and keeps it up to date after that.

## Test locally on Arch

```sh
packaging/aur/render.sh 0.2.0 <sha256 of bawkseek-v0.2.0-linux-x64.tar.gz> /tmp/bawkseek-bin
cd /tmp/bawkseek-bin && makepkg -si
```
