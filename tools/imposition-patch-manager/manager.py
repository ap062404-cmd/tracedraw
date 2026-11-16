#!/usr/bin/env python3
"""Small, dependency-free manager for the isolated TraceDraw Imposition DEB."""
from __future__ import annotations
import argparse, hashlib, json, os, shutil, subprocess, sys, tempfile
from pathlib import Path
from datetime import datetime, timezone

PACKAGE = "tracedraw-imposition"
ARCH = "amd64"
STATE_DIR = Path("/var/lib/tracedraw-imposition-manager")
VERSIONS_DIR = STATE_DIR / "packages"
MANIFEST = STATE_DIR / "history.json"
EXPECTED_FILES = {
    "/usr/bin/tracedraw-imposition",
    "/usr/lib/tracedraw-imposition/tracedraw",
    "/usr/lib/tracedraw-imposition/tracedraw-cli",
    "/usr/share/applications/tracedraw-imposition.desktop",
    "/usr/share/icons/hicolor/256x256/apps/tracedraw-imposition.png",
}
FORBIDDEN_FILES = {
    "/usr/bin/tracedraw", "/usr/bin/tracedraw-cli",
    "/usr/share/applications/tracedraw.desktop",
    "/usr/share/mime/packages/tracedraw.xml",
    "/usr/share/thumbnailers/tracedraw.thumbnailer",
}

class ManagerError(Exception):
    pass

def run(args: list[str], check: bool = True) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(args, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    except FileNotFoundError as exc:
        raise ManagerError(f"Perintah tidak ditemukan: {args[0]}") from exc
    if check and result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        raise ManagerError(f"Perintah gagal ({result.returncode}): {' '.join(args)}\n{detail}")
    return result

def deb_field(deb: Path, field: str) -> str:
    return run(["dpkg-deb", "-f", str(deb), field]).stdout.strip()

def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def validate_deb(deb: Path) -> tuple[str, str]:
    if not deb.is_file() or deb.suffix != ".deb":
        raise ManagerError("Pilih satu file .deb yang tersedia.")
    name, version, arch = (deb_field(deb, f) for f in ("Package", "Version", "Architecture"))
    if name != PACKAGE:
        raise ManagerError(f"Paket salah: ditemukan '{name}', diharapkan '{PACKAGE}'.")
    if arch != ARCH:
        raise ManagerError(f"Arsitektur '{arch}' tidak didukung; manager ini untuk {ARCH}.")
    listing = run(["dpkg-deb", "-c", str(deb)]).stdout
    paths = set()
    for line in listing.splitlines():
        parts = line.split()
        if parts:
            candidate = parts[-1]
            if candidate.startswith("./"):
                candidate = candidate[1:]
            if candidate.startswith("/"):
                paths.add(candidate.rstrip("/"))
    missing, conflicts = EXPECTED_FILES - paths, FORBIDDEN_FILES & paths
    if missing:
        raise ManagerError("Isi paket tidak lengkap: " + ", ".join(sorted(missing)))
    if conflicts:
        raise ManagerError("Paket berisi file yang dapat menimpa TraceDraw resmi: " +
                           ", ".join(sorted(conflicts)))
    return version, sha256(deb)

def installed_version() -> str | None:
    result = run(["dpkg-query", "-W", "-f=$"+"{Status}|$"+"{Version}", PACKAGE], check=False)
    if result.returncode:
        return None
    status, _, version = result.stdout.strip().partition("|")
    return version if status == "install ok installed" else None

def load_history() -> dict:
    if not MANIFEST.exists():
        return {"package": PACKAGE, "installs": []}
    try:
        data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ManagerError(f"Riwayat manager rusak: {exc}") from exc
    if data.get("package") != PACKAGE or not isinstance(data.get("installs"), list):
        raise ManagerError("Format riwayat manager tidak dikenali.")
    return data

def save_history(data: dict) -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix="history-", dir=STATE_DIR)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(data, stream, indent=2)
            stream.write("\n")
        os.chmod(temp_name, 0o644)
        os.replace(temp_name, MANIFEST)
    finally:
        if os.path.exists(temp_name):
            os.unlink(temp_name)

def require_root() -> None:
    if os.geteuid() != 0:
        raise ManagerError("Aksi ini membutuhkan hak administrator. Jalankan dengan sudo.")

def status() -> None:
    version, history = installed_version(), load_history()
    installs = history["installs"]
    managed = bool(version and history.get("active_version") == version and
                   any(item.get("version") == version for item in installs))
    print(f"Paket            : {PACKAGE}")
    print(f"Status           : {'terpasang' if version else 'belum terpasang'}")
    print(f"Versi terpasang  : {version or '-'}")
    print(f"Dikelola manager : {'ya' if managed else ('tidak' if version else '-')}")
    print(f"Versi tersimpan  : {len(installs)}")
    print("Lokasi aplikasi  : /usr/lib/tracedraw-imposition/tracedraw")
    print("Konfigurasi      : ~/.config/tracedraw-imposition")

def install(deb: Path) -> None:
    require_root()
    deb = deb.resolve()
    version, digest = validate_deb(deb)
    current, history = installed_version(), load_history()
    known = {item.get("version") for item in history["installs"]}
    if current and current not in known:
        raise ManagerError(
            f"Versi {current} terpasang tetapi bukan melalui manager. "
            "Manager menolak update agar rollback tidak kehilangan paket versi lama."
        )
    if current == version:
        print(f"Versi {version} sudah terpasang; tidak ada perubahan.")
        return
    VERSIONS_DIR.mkdir(parents=True, exist_ok=True)
    stored = VERSIONS_DIR / f"{PACKAGE}_{version}_{ARCH}.deb"
    if stored.exists() and sha256(stored) != digest:
        raise ManagerError("Versi sama sudah tersimpan dengan checksum berbeda.")
    existing = next((item for item in history["installs"]
                     if item.get("version") == version), None)
    if existing and (existing.get("sha256") != digest or existing.get("file") != str(stored)):
        raise ManagerError("Metadata versi tersimpan tidak cocok dengan paket ini.")
    if not stored.exists():
        shutil.copy2(deb, stored)
    run(["dpkg", "-i", str(stored)])
    # Keep the list in installation order. Reinstalling an older saved version
    # moves it to the end so rollback always selects the immediately prior version.
    history["installs"] = [
        item for item in history["installs"] if item.get("version") != version
    ]
    history["installs"].append({
        "version": version, "sha256": digest, "file": str(stored),
        "installed_at": datetime.now(timezone.utc).isoformat(),
    })
    history["active_version"] = version
    save_history(history)
    print(f"Berhasil memasang TraceDraw Imposition {version}.")

def rollback() -> None:
    require_root()
    current = installed_version()
    if not current:
        raise ManagerError("TraceDraw Imposition belum terpasang.")
    history = load_history()
    installs = history["installs"]
    active = history.get("active_version")
    if active != current:
        raise ManagerError("Versi terpasang tidak cocok dengan versi aktif dalam riwayat manager.")
    active_index = next((i for i, item in enumerate(installs)
                         if item.get("version") == active), None)
    if active_index is None or active_index == 0:
        raise ManagerError("Belum ada versi sebelumnya yang tersimpan untuk rollback.")
    previous = installs[active_index - 1]
    package_file = Path(previous.get("file", ""))
    if not package_file.is_file() or sha256(package_file) != previous.get("sha256"):
        raise ManagerError("Paket versi sebelumnya hilang atau checksum tidak cocok.")
    version, _ = validate_deb(package_file)
    if version != previous["version"]:
        raise ManagerError("Versi paket tersimpan tidak sesuai riwayat.")
    run(["dpkg", "-i", str(package_file)])
    history["active_version"] = version
    save_history(history)
    print(f"Rollback berhasil: {current} -> {version}.")

def main() -> int:
    parser = argparse.ArgumentParser(description="Patch Manager ringan untuk TraceDraw Imposition.")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("status", help="Periksa status instalasi.")
    install_parser = sub.add_parser("install", help="Validasi lalu pasang file DEB.")
    install_parser.add_argument("deb", type=Path, help="Lokasi file .deb")
    sub.add_parser("rollback", help="Kembali ke versi sebelumnya yang dikelola manager.")
    args = parser.parse_args()
    try:
        if args.command == "status": status()
        elif args.command == "install": install(args.deb)
        elif args.command == "rollback": rollback()
        return 0
    except ManagerError as exc:
        print(f"Kesalahan: {exc}", file=sys.stderr)
        return 1

if __name__ == "__main__":
    raise SystemExit(main())
