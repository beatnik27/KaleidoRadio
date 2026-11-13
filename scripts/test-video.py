#!/usr/bin/env python3
"""Exercise native video with local, silent, authenticated HTTP and HLS sources."""
import base64
import functools
import http.server
import pathlib
import subprocess
import threading
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "target/video-tests"
OUT.mkdir(parents=True, exist_ok=True)

def run(*args):
    subprocess.run(args, cwd=ROOT, check=True)

run("ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100", "-t", "5", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-movflags", "+faststart", str(OUT / "sample.mp4"))
run("ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=360x640:rate=24", "-t", "5", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-an", "-movflags", "+faststart", str(OUT / "silent.mp4"))
run("ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", str(OUT / "sample.mp4"), "-c", "copy", "-hls_time", "1", "-hls_list_size", "0", "-hls_playlist_type", "vod", str(OUT / "sample.m3u8"))
run("ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", str(OUT / "sample.mp4"), "-vn", "-c:a", "copy", str(OUT / "audio-only.mp4"))
frameworks = ["Foundation", "AVFoundation", "CoreVideo", "CoreImage", "CoreMedia", "QuartzCore", "CoreGraphics"]
run("xcrun", "clang", "-fobjc-arc", "-fblocks", "-mmacosx-version-min=12.0", "tests/native_video.m", *(arg for framework in frameworks for arg in ("-framework", framework)), "-o", str(OUT / "probe"))
for name in (() if "--hls-only" in sys.argv else ("sample.mp4", "silent.mp4")):
    run(str(OUT / "probe"), (OUT / name).as_uri())

if "--hls-only" not in sys.argv:
    run(str(OUT / "probe"), (OUT / "audio-only.mp4").as_uri(), "--audio-only")

class MediaServer(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        if self.path.startswith("/auth/"):
            if self.headers.get("Authorization") != "Basic " + base64.b64encode(b"video-test:password").decode():
                self.send_response(401)
                self.send_header("WWW-Authenticate", 'Basic realm="Video test"')
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            self.path = self.path.removeprefix("/auth")
        path = pathlib.Path(self.translate_path(self.path))
        if not path.is_file():
            return super().do_GET()
        data = path.read_bytes()
        start, end = 0, len(data) - 1
        if self.headers.get("Range"):
            span = self.headers["Range"].removeprefix("bytes=").split("-", 1)
            start = int(span[0] or 0)
            end = min(int(span[1]) if span[1] else end, end)
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        else:
            self.send_response(200)
        self.send_header("Content-Type", self.guess_type(str(path)))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(end - start + 1))
        self.end_headers()
        try:
            self.wfile.write(data[start:end + 1])
        except (BrokenPipeError, ConnectionResetError):
            pass

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(MediaServer, directory=str(OUT)))
threading.Thread(target=server.serve_forever, daemon=True).start()
try:
    base = f"http://127.0.0.1:{server.server_port}"
    if "--hls-only" not in sys.argv:
        run(str(OUT / "probe"), base + "/sample.mp4")
        run(str(OUT / "probe"), base + "/auth/sample.mp4", "video-test", "password")
    run(str(OUT / "probe"), base + "/sample.m3u8")
finally:
    server.shutdown()
print("Selected native video source checks passed." if "--hls-only" in sys.argv else "All native video source checks passed.")
