// Gate 0 for the iOS port: SDL3 + OpenGL ES 2.0 in the simulator, one
// glDrawElements triangle per frame. Apple Silicon simulators have a
// reported crash inside glDrawElements_ES2Exec (Apple forums 756598,
// FB13850673), and raylib's ES2 batch renderer is built on that call, so
// this runs before any game work and again after every Xcode or runtime
// update. Built and launched by tools/ios/smoke.sh (`just ios-smoke`).
#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h> // on iOS this supplies main() -> SDL_RunApp -> SDL_main
#include <OpenGLES/ES2/gl.h>
#include <stdlib.h>

#define FRAMES 300

static const char *VS = "attribute vec2 p; void main() { gl_Position = vec4(p, 0.0, 1.0); }";
static const char *FS = "precision mediump float; void main() { gl_FragColor = vec4(1.0, 0.5, 0.0, 1.0); }";

static GLuint compile(GLenum type, const char *src)
{
    GLuint sh = glCreateShader(type);
    glShaderSource(sh, 1, &src, NULL);
    glCompileShader(sh);
    GLint ok = 0;
    glGetShaderiv(sh, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        char log[512];
        glGetShaderInfoLog(sh, sizeof log, NULL, log);
        SDL_Log("SMOKE FAIL shader: %s", log);
        exit(2);
    }
    return sh;
}

int main(int argc, char **argv)
{
    (void)argc; (void)argv;
    if (!SDL_Init(SDL_INIT_VIDEO)) { SDL_Log("SMOKE FAIL SDL_Init: %s", SDL_GetError()); return 1; }
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_PROFILE_MASK, SDL_GL_CONTEXT_PROFILE_ES);
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_MAJOR_VERSION, 2);
    SDL_GL_SetAttribute(SDL_GL_CONTEXT_MINOR_VERSION, 0);
    SDL_Window *win = SDL_CreateWindow("smoke", 960, 512, SDL_WINDOW_OPENGL | SDL_WINDOW_FULLSCREEN);
    if (!win) { SDL_Log("SMOKE FAIL window: %s", SDL_GetError()); return 1; }
    SDL_GLContext ctx = SDL_GL_CreateContext(win);
    if (!ctx) { SDL_Log("SMOKE FAIL context: %s", SDL_GetError()); return 1; }
    SDL_GL_SetSwapInterval(1);
    int w = 0, h = 0, pw = 0, ph = 0;
    SDL_GetWindowSize(win, &w, &h);
    SDL_GetWindowSizeInPixels(win, &pw, &ph);
    SDL_Log("GL_VERSION=%s GL_RENDERER=%s window=%dx%d pixels=%dx%d",
            glGetString(GL_VERSION), glGetString(GL_RENDERER), w, h, pw, ph);

    GLuint prog = glCreateProgram();
    glAttachShader(prog, compile(GL_VERTEX_SHADER, VS));
    glAttachShader(prog, compile(GL_FRAGMENT_SHADER, FS));
    glBindAttribLocation(prog, 0, "p");
    glLinkProgram(prog);
    glUseProgram(prog);
    const float verts[] = { -0.6f, -0.5f, 0.6f, -0.5f, 0.0f, 0.6f };
    const unsigned short idx[] = { 0, 1, 2 };
    GLuint vbo = 0, ibo = 0;
    glGenBuffers(1, &vbo);
    glBindBuffer(GL_ARRAY_BUFFER, vbo);
    glBufferData(GL_ARRAY_BUFFER, sizeof verts, verts, GL_STATIC_DRAW);
    glGenBuffers(1, &ibo);
    glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
    glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof idx, idx, GL_STATIC_DRAW);
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, 0);

    for (int frame = 0; frame < FRAMES; frame++) {
        SDL_Event e;
        while (SDL_PollEvent(&e)) {
            if (e.type == SDL_EVENT_FINGER_DOWN) SDL_Log("finger %.3f %.3f", e.tfinger.x, e.tfinger.y);
            if (e.type == SDL_EVENT_QUIT) return 0;
        }
        glViewport(0, 0, pw, ph);
        glClearColor(0.1f, 0.2f, 0.3f, 1.0f);
        glClear(GL_COLOR_BUFFER_BIT);
        glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, 0); // the call the simulator bug lives in
        if (frame % 60 == 0) SDL_Log("frame %d glGetError=0x%x", frame, glGetError());
        SDL_GL_SwapWindow(win);
    }
    SDL_Log("SMOKE OK");
    return 0;
}
