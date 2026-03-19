# Preparing Multi-Channel Tracks with merge_tracks.sh

The `merge_tracks.sh` script helps you combine multiple mono or stereo audio files into a single multi-channel file.

## Setup

1. Create a `merge/` directory in the project root
2. Create subdirectories inside `merge/` — each subdirectory represents one track
3. Place your audio files in each subdirectory

Example structure:
```
merge/
├── song_1/
│   ├── kick.wav
│   ├── snare.wav
│   ├── hihat.wav
│   ├── bass.wav
│   ├── guitar_L.wav
│   ├── guitar_R.wav
│   ├── vocal_L.wav
│   └── vocal_R.wav
└── song_2/
    ├── drums.wav
    ├── bass.wav
    ├── keys_L.wav
    └── keys_R.wav
```

## Running the script

```bash
chmod +x merge_tracks.sh
./merge_tracks.sh
```

The script will:
- Process each subdirectory in `merge/`
- Combine all audio files in each folder into a single multi-channel file
- Output the merged files to `tracks/` directory
- Preserve the format (WAV or FLAC) from the input files
- Show a summary of successful, skipped, and failed merges

**Notes:**
- Files are merged in alphabetical order
- Each subdirectory must contain at least 2 audio files
- All files in a folder should have the same sample rate and bit depth
- The output will have as many channels as the sum of all input files

## Adding metadata

To add artist and title metadata to your tracks, use `ffmpeg`:

```bash
ffmpeg -i input.wav -metadata artist="Artist Name" -metadata title="Track Title" -codec copy output.wav
```
