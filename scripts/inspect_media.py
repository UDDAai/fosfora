#!/usr/bin/env python3
"""Read-only video preflight for Fosfora's all-frames-in-RAM decoder.

The script diagnoses metadata and budget limitations, not GPU playback.
No files are copied, deleted, hydrated intentionally, or re-encoded.
"""
import argparse
import json
import math
import shutil
import subprocess
import sys
from pathlib import Path

VIDEO_EXTENSIONS = {".mp4", ".mov", ".avi", ".mkv", ".webm", ".m4v", ".flv"}
MAX_CLIP_SECONDS = 60.0
MAX_CLIP_BYTES = 2 * 1024 ** 3
MAX_BROWSER_UPLOAD_BYTES = 512 * 1024 ** 2


def rate(raw):
    try:
        if not raw:
            return None
        num, den = raw.split("/", 1) if "/" in raw else (raw, "1")
        n, d = float(num), float(den)
        value = n / d if d else 0
        return value if math.isfinite(value) and value > 0 else None
    except (ValueError, ZeroDivisionError):
        return None


def positive_float(raw):
    try:
        number = float(raw)
        return number if math.isfinite(number) and number > 0 else None
    except (TypeError, ValueError):
        return None


def probe(file, ffprobe):
    try:
        result = subprocess.run(
            [
                ffprobe, "-v", "error", "-print_format", "json",
                "-show_streams", "-show_format", str(file),
            ],
            capture_output=True, text=True, timeout=30, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return None, "ffprobe failed: " + str(exc)
    if result.returncode:
        return None, result.stderr.strip() or f"ffprobe exit {result.returncode}"
    try:
        info = json.loads(result.stdout)
    except ValueError as exc:
        return None, f"invalid ffprobe JSON: {exc}"
    streams = info.get("streams") or []
    stream = next((s for s in streams if s.get("codec_type") == "video"), None)
    if stream is None:
        return None, "no video stream found"

    width, height = stream.get("width", 0), stream.get("height", 0)
    if not isinstance(width, int) or not isinstance(height, int) or min(width, height) <= 0:
        return None, "missing/invalid dimensions"
    fps = rate(stream.get("avg_frame_rate")) or rate(stream.get("r_frame_rate"))
    if fps is None:
        return None, "missing/invalid frame rate (0/0)"
    frames = stream.get("nb_frames")
    try:
        frames = int(frames) if frames is not None and int(frames) > 0 else None
    except (ValueError, TypeError):
        frames = None
    fmt = info.get("format") or {}
    duration = positive_float(fmt.get("duration")) or positive_float(stream.get("duration"))
    duration = duration or (frames / fps if frames else None)
    if duration is None:
        return None, "missing duration and frame count"
    count = frames or math.ceil(duration * fps)
    decoded = width * height * count * 4
    errors = []
    if duration > MAX_CLIP_SECONDS:
        errors.append(f"{duration:.1f}s exceeds {MAX_CLIP_SECONDS:g}s pre-decode limit")
    if decoded > MAX_CLIP_BYTES:
        errors.append(f"{decoded / 1024**3:.2f} GiB decoded exceeds 2 GiB per-clip limit")
    return {
        "codec": stream.get("codec_name", "?"),
        "pixel_format": stream.get("pix_fmt", "?"),
        "width": width,
        "height": height,
        "fps": fps,
        "duration": duration,
        "frames": count,
        "decoded": decoded,
        "upload_warning": file.stat().st_size > MAX_BROWSER_UPLOAD_BYTES,
    }, "; ".join(errors)


def main():
    parser = argparse.ArgumentParser(
        description="Diagnose Fosfora-compatible video files without changing media"
    )
    parser.add_argument("paths", nargs="+", type=Path, help="Video files or folders")
    parser.add_argument("--ffprobe", default=shutil.which("ffprobe"),
                        help="Absolute path to the ffprobe executable")
    args = parser.parse_args()
    if not args.ffprobe:
        parser.error("ffprobe was not found; pass --ffprobe /path/to/ffprobe")
    files = []
    for path in args.paths:
        if path.is_dir():
            files.extend(p for p in path.rglob("*")
                         if p.is_file() and p.suffix.lower() in VIDEO_EXTENSIONS)
        else:
            files.append(path)
    files = sorted(set(files))
    if not files:
        print("No supported video extensions found.", file=sys.stderr)
        return 1
    failures = 0
    for path in files:
        if path.suffix.lower() not in VIDEO_EXTENSIONS:
            print(f"SKIP {path}: unsupported extension")
            failures += 1
            continue
        meta, error = probe(path, args.ffprobe)
        if meta is None:
            failures += 1
            print(f"FAIL {path}: {error}")
            continue
        status = "FAIL" if error else "OK"
        failures += bool(error)
        print(
            f"{status} {path.name} | {meta['codec']} / {meta['pixel_format']} | "
            f"{meta['width']}x{meta['height']} {meta['fps']:.2f}fps "
            f"{meta['duration']:.1f}s {meta['frames']} frames | "
            f"decoded {meta['decoded'] / 1024**2:.0f} MiB"
        )
        if error:
            print("   " + error)
        if meta["upload_warning"]:
            print("   Note: compressed file >512 MiB; browser upload will refuse it")
    print(f"Checked {len(files)} video(s); {failures} failed metadata/limit checks.")
    print("An OK result does not guarantee that the video codec decodes successfully.")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
