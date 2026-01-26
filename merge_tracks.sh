#!/bin/bash

# Script to merge multi-channel audio files into single tracks
# Usage: ./merge_tracks.sh
#
# Automatically processes all folders in merge/
# Each folder becomes one merged track in tracks/

MERGE_BASE="merge"
TRACKS_DIR="tracks"

# Check if merge directory exists
if [ ! -d "$MERGE_BASE" ]; then
    echo "Error: merge/ directory not found"
    echo "Create a merge/ directory and add folders with audio files to merge"
    exit 1
fi

# Create tracks directory if it doesn't exist
mkdir -p "$TRACKS_DIR"

# Find all subdirectories in merge/
mapfile -t TRACK_FOLDERS < <(find "$MERGE_BASE" -mindepth 1 -maxdepth 1 -type d | sort)

if [ ${#TRACK_FOLDERS[@]} -eq 0 ]; then
    echo "No track folders found in merge/"
    exit 0
fi

echo "Found ${#TRACK_FOLDERS[@]} track folder(s) to process"
echo "=================================="
echo ""

SUCCESS_COUNT=0
SKIP_COUNT=0
FAIL_COUNT=0

for MERGE_DIR in "${TRACK_FOLDERS[@]}"; do
    TRACK_NAME=$(basename "$MERGE_DIR")

    echo "Processing: $TRACK_NAME"
    echo "---"

    # Find all audio files in this directory, sorted alphabetically
    mapfile -t AUDIO_FILES < <(find "$MERGE_DIR" -maxdepth 1 -type f \( -iname "*.wav" -o -iname "*.flac" -o -iname "*.mp3" \) | sort)

    if [ ${#AUDIO_FILES[@]} -eq 0 ]; then
        echo "  Skipping: No audio files found"
        echo ""
        ((SKIP_COUNT++))
        continue
    fi

    if [ ${#AUDIO_FILES[@]} -lt 2 ]; then
        echo "  Skipping: Only ${#AUDIO_FILES[@]} file found (need at least 2 to merge)"
        echo ""
        ((SKIP_COUNT++))
        continue
    fi

    # Get file extension from first input to determine format
    FIRST_FILE="${AUDIO_FILES[0]}"
    EXT="${FIRST_FILE##*.}"
    EXT_LOWER=$(echo "$EXT" | tr '[:upper:]' '[:lower:]')

    # Determine output format and codec
    if [ "$EXT_LOWER" = "flac" ]; then
        CODEC="flac"
        OUTPUT_EXT="flac"
        echo "  Format: FLAC"
    elif [ "$EXT_LOWER" = "wav" ]; then
        CODEC="pcm_s24le"
        OUTPUT_EXT="wav"
        echo "  Format: WAV (24-bit PCM)"
    else
        echo "  Warning: Unrecognized format '$EXT_LOWER', defaulting to WAV"
        CODEC="pcm_s24le"
        OUTPUT_EXT="wav"
    fi

    OUTPUT="$TRACKS_DIR/${TRACK_NAME}.${OUTPUT_EXT}"

    echo "  Files to merge: ${#AUDIO_FILES[@]}"
    for i in "${!AUDIO_FILES[@]}"; do
        echo "    [$((i+1))] $(basename "${AUDIO_FILES[$i]}")"
    done
    echo "  Output: $OUTPUT"

    # Build ffmpeg command with all input files
    FFMPEG_CMD="ffmpeg -y"
    FILTER_INPUTS=""

    for i in "${!AUDIO_FILES[@]}"; do
        FFMPEG_CMD="$FFMPEG_CMD -i \"${AUDIO_FILES[$i]}\""
        FILTER_INPUTS="${FILTER_INPUTS}[$i:a]"
    done

    # Build the filter complex to merge all audio streams
    FILTER_COMPLEX="${FILTER_INPUTS}amerge=inputs=${#AUDIO_FILES[@]}[aout]"

    # Execute ffmpeg command
    eval "$FFMPEG_CMD -filter_complex \"$FILTER_COMPLEX\" -map \"[aout]\" -c:a \"$CODEC\" \"$OUTPUT\"" 2>/dev/null

    if [ $? -eq 0 ]; then
        # Get channel count for the output
        CHANNELS=$(ffprobe -v error -select_streams a:0 -show_entries stream=channels -of default=noprint_wrappers=1:nokey=1 "$OUTPUT" 2>/dev/null)
        echo "  Success! Created ${CHANNELS}-channel file"
        ((SUCCESS_COUNT++))
    else
        echo "  Failed to merge files"
        ((FAIL_COUNT++))
    fi

    echo ""
done

echo "=================================="
echo "Summary:"
echo "  Success: $SUCCESS_COUNT"
echo "  Skipped: $SKIP_COUNT"
echo "  Failed:  $FAIL_COUNT"
