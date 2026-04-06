#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" || $# -lt 2 ]]; then
  echo "Usage: $0 <input_dir> <output_dir> [workers] [bitrate_kbps]"
  echo "Example: $0 \"Music Folder\" \"Music Folder Test ffmpeg\" 10 128"
  exit 1
fi

INPUT_DIR="$1"
OUTPUT_DIR="$2"
WORKERS="${3:-$(sysctl -n hw.ncpu)}"
BITRATE_KBPS="${4:-128}"

if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "ffmpeg not found in PATH"
  exit 1
fi

if [[ ! -d "$INPUT_DIR" ]]; then
  echo "Input directory does not exist: $INPUT_DIR"
  exit 1
fi

if [[ "$WORKERS" -le 0 ]]; then
  echo "workers must be > 0"
  exit 1
fi

if [[ "$BITRATE_KBPS" -le 0 ]]; then
  echo "bitrate_kbps must be > 0"
  exit 1
fi

if [[ -d "$OUTPUT_DIR" ]]; then
  rm -rf "$OUTPUT_DIR"
fi
mkdir -p "$OUTPUT_DIR"

TMP_LIST="$(mktemp)"
trap 'rm -f "$TMP_LIST"' EXIT

find "$INPUT_DIR" -type f -iname "*.mp3" -print0 > "$TMP_LIST"
FILE_COUNT="$(tr -cd '\000' < "$TMP_LIST" | wc -c | tr -d ' ')"

if [[ "$FILE_COUNT" -eq 0 ]]; then
  echo "No .mp3 files found under: $INPUT_DIR"
  exit 1
fi

echo "FFmpeg folder transcode starting"
echo "input_dir=$INPUT_DIR"
echo "output_dir=$OUTPUT_DIR"
echo "workers=$WORKERS"
echo "bitrate_kbps=$BITRATE_KBPS"
echo "files=$FILE_COUNT"

TIME_OUTPUT="$(
  {
    /usr/bin/time -p xargs -0 -P "$WORKERS" -I {} bash -c '
      set -euo pipefail
      in="$1"
      input_root="$2"
      output_root="$3"
      bitrate_kbps="$4"

      rel="${in#"$input_root"/}"
      out="$output_root/${rel%.*}.aac"
      mkdir -p "$(dirname "$out")"

      ffmpeg -nostdin -hide_banner -loglevel error -y -threads 1 \
        -i "$in" -vn -c:a aac -b:a "${bitrate_kbps}k" "$out"
    ' _ {} "$INPUT_DIR" "$OUTPUT_DIR" "$BITRATE_KBPS" < "$TMP_LIST"
  } 2>&1
)"

OUTPUT_COUNT="$(find "$OUTPUT_DIR" -type f -iname "*.aac" | wc -l | tr -d ' ')"
INPUT_BYTES="$(find "$INPUT_DIR" -type f -iname "*.mp3" -print0 | xargs -0 stat -f%z | awk '{s+=$1} END {print s+0}')"
OUTPUT_BYTES="$(find "$OUTPUT_DIR" -type f -iname "*.aac" -print0 | xargs -0 stat -f%z | awk '{s+=$1} END {print s+0}')"

REAL_SECONDS="$(awk '$1=="real"{print $2}' <<< "$TIME_OUTPUT")"

echo
echo "Results"
echo "processed=$FILE_COUNT"
echo "succeeded=$OUTPUT_COUNT"
echo "failed=$((FILE_COUNT - OUTPUT_COUNT))"
echo "input_mib=$(awk -v b="$INPUT_BYTES" 'BEGIN{printf "%.2f", b/1048576}')"
echo "output_mib=$(awk -v b="$OUTPUT_BYTES" 'BEGIN{printf "%.2f", b/1048576}')"
echo "elapsed_s=$REAL_SECONDS"
echo "files_per_second=$(awk -v f="$FILE_COUNT" -v t="$REAL_SECONDS" 'BEGIN{if(t>0) printf "%.2f", f/t; else print "inf"}')"
echo "input_mib_per_second=$(awk -v b="$INPUT_BYTES" -v t="$REAL_SECONDS" 'BEGIN{if(t>0) printf "%.2f", (b/1048576)/t; else print "inf"}')"
