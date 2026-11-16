# TraceDraw Imposition Patch Manager

Lightweight Linux command-line manager for the isolated `tracedraw-imposition` DEB.
It does not patch or overwrite the official `tracedraw` package.

## Requirements
- Debian/Ubuntu-based Linux (including Linux Mint)
- Python 3.10+
- `dpkg` and `dpkg-deb`
- amd64 DEB packages produced by the isolated Imposition packaging workflow

## Commands

Check installation:
```bash
python3 manager.py status
```

Install or update:
```bash
sudo python3 manager.py install ./tracedraw-imposition_VERSION_amd64.deb
```

Roll back to the preceding version installed through this manager:
```bash
sudo python3 manager.py rollback
```

## Safety model
- Checks package name, architecture, required files, and SHA-256 before install.
- Rejects packages containing known official TraceDraw executable, desktop, MIME,
  or thumbnailer paths.
- Stores managed DEBs under `/var/lib/tracedraw-imposition-manager/packages/`.
- Keeps JSON installation history and only rolls back to a stored package whose
  checksum still matches.
- Refuses to update an installation whose current version was not installed
  through this manager, because safe rollback cannot otherwise be guaranteed.
- Uses the isolated launcher and `~/.config/tracedraw-imposition`.

This first release is a CLI foundation. A graphical front end, online release
feed, backup export/import, and signed release verification are not implemented.
SHA-256 is checked against local history; it is not a publisher signature.
