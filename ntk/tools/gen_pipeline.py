"""Build-pipeline diagram for the NTK slides: ntk/assets/build-pipeline.svg
(+ .png at 4x, 7680 x 4320). White on black, no text but the diagram's.

    PYTHONPATH=ntk/tools nix-shell -p librsvg --run "python3 ntk/tools/gen_pipeline.py"
"""
from sketch import Sketch

s = Sketch()
s.box(60, 380, 300, 170, ["src/  edition 2024", "sola-raylib · rapier2d · hecs", "static/ assets · maps/"], title="Rust source")
s.box(60, 590, 300, 110, ["one build.rs picks the", "link flags per target"], dashed=True)
s.arrow(360, 465, 450, 465)
s.box(450, 385, 262, 160, ["rustc frontend →", "LLVM IR → machine code", "for the target triple"], title="LLVM", size=20)

cols = [786, 1070, 1354, 1638]
BW, BH = 262, 130
rows = [
    ("DESKTOP", 180, [
        ["x86_64 / aarch64", "macOS · Linux · Windows"],
        ["cmake builds raylib (GLFW),", "cc links the binary"],
        ["bongbong binary", "+ static/ beside it"],
        ["cargo-dist archives", "→ GitHub Releases"]]),
    ("WEB", 400, [
        ["wasm32-unknown-emscripten"],
        ["emcc (pinned emsdk) links", "raylib PLATFORM=Web,", "--preload-file static/"],
        [".wasm + .js glue + .data", "GLSL ES 100 shaders"],
        ["Astro site →", "Cloudflare Workers", "bongbong.io"]]),
    ("iOS", 620, [
        ["aarch64-apple-ios / -sim"],
        ["clang links prebuilt raylib", "(SDL3 backend, GL ES 2)", "+ libSDL3.a + frameworks"],
        ["bundle.sh → BongBong.app", "Info.plist · static/ · maps/", "no Xcode project"],
        ["Xcode automatic signing", "→ devicectl / Simulator"]]),
    ("ANDROID", 840, [
        ["aarch64-linux-android", "cdylib crate android/"],
        ["NDK clang links prebuilt", "raylib (NativeActivity)", "+ EGL · GLESv2, --wrap=fopen"],
        ["libbongbong_android.so", "+ assets → aapt2 + apksigner", "→ APK, no Java"],
        ["adb install", "→ device / AVD"]]),
]
BUS = 740
s.line(712, 465, BUS, 465)
s.line(BUS, rows[0][1] + BH / 2, BUS, rows[-1][1] + BH / 2)
for name, y, boxes in rows:
    cy = y + BH / 2
    s.text(cols[0], y - 16, name, 20, 700)
    s.arrow(BUS, cy, cols[0], cy)
    for i, lines in enumerate(boxes):
        s.box(cols[i], y, BW, BH, lines, size=18)
        if i < 3:
            s.arrow(cols[i] + BW, cy, cols[i + 1], cy)

s.save("ntk/assets/build-pipeline.svg", "ntk/assets/build-pipeline.png")
