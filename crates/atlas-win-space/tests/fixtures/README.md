# Native property-store fixtures

Synthetic media created for these tests; no third-party content or personal metadata.
The tests embed the files and do not require FFmpeg at runtime.

Regenerate from the repository root with FFmpeg:

```powershell
ffmpeg -f lavfi -i anullsrc=r=44100:cl=mono -t 1 -c:a libmp3lame -b:a 128k -metadata artist="Atlas Metadata Fixture" -y crates/atlas-win-space/tests/fixtures/metadata.mp3
ffmpeg -f lavfi -i color=c=black:s=32x24:r=10 -t 1 -c:v libx264 -pix_fmt yuv420p -movflags +faststart -y crates/atlas-win-space/tests/fixtures/metadata.mp4
```
