// Gate 0 for the Android port: raylib's own Android platform in a
// NativeActivity, before any Rust. Clears the screen, draws a moving
// rectangle and text, loads one asset from the APK (the same fopen wrap the
// game relies on) and logs every touch, so one logcat capture proves the
// toolchain, the packaging, GL ES 2 on the emulator, asset access and
// input. Built and launched by tools/android/smoke.sh (`just android-smoke`).
#include "raylib.h"
#include <android/log.h>

int main(int argc, char *argv[])
{
    (void)argc; (void)argv;
    InitWindow(0, 0, "smoke");
    const int w = GetScreenWidth(), h = GetScreenHeight();
    __android_log_print(ANDROID_LOG_INFO, "smoke", "screen %dx%d render %dx%d", w, h, GetRenderWidth(), GetRenderHeight());
    Texture2D tex = LoadTexture("static/pickups/health.png");
    __android_log_print(ANDROID_LOG_INFO, "smoke", "asset static/pickups/health.png: %dx%d (0x0 = asset load failed)", tex.width, tex.height);
    int frame = 0;
    while (!WindowShouldClose()) {
        int touches = GetTouchPointCount();
        for (int i = 0; i < touches; i++) {
            Vector2 p = GetTouchPosition(i);
            __android_log_print(ANDROID_LOG_INFO, "smoke", "touch %d at %.0f,%.0f (frame %d)", i, p.x, p.y, frame);
        }
        BeginDrawing();
        ClearBackground((Color){ 26, 51, 77, 255 });
        DrawRectangle((frame * 4) % w, h / 3, 200, 120, ORANGE);
        DrawTextureEx(tex, (Vector2){ 40, 40 }, 0.0f, 4.0f, WHITE);
        DrawText("SMOKE OK", w / 2 - 200, h / 2, 80, RAYWHITE);
        if (touches > 0) DrawCircleV(GetTouchPosition(0), 60, RED);
        EndDrawing();
        if (frame % 300 == 0) __android_log_print(ANDROID_LOG_INFO, "smoke", "frame %d fps %d", frame, GetFPS());
        frame++;
    }
    UnloadTexture(tex);
    CloseWindow();
    return 0;
}
