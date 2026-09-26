# Security

Report problems privately through GitHub's "Report a vulnerability" on this repository.

Every released `reforge.dll` is built by GitHub Actions from a tagged commit. Check that it came from there:

    gh attestation verify reforge.dll --repo Vansinnet/Reforge

and compare its SHA-256 with the release's `SHA256SUMS`. Do not run copies that fail either check.
