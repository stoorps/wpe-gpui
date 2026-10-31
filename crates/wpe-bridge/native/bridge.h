#pragma once
#include <stdint.h>
typedef struct BrowserView BrowserView;
BrowserView *bw_create(void *display, void *surface, char **error);
void bw_destroy(BrowserView *);
void bw_free(void *);
void bw_pump(void);
void bw_geometry(BrowserView *, int width, int height, double scale,
                 int visible);
void bw_focus(BrowserView *, int);
void bw_load(BrowserView *, const char *uri);
void bw_load_html(BrowserView *, const char *html, const char *base, int email);
void bw_action(BrowserView *, int action);
void bw_zoom(BrowserView *, double);
const char *bw_uri(BrowserView *);
const char *bw_title(BrowserView *);
const char *bw_error(BrowserView *);
char *bw_take_new_uri(BrowserView *);
int bw_state(BrowserView *);
uint64_t bw_frames(BrowserView *);
void bw_pointer(BrowserView *, int kind, double x, double y, int button,
                uint32_t mods, int count);
void bw_scroll(BrowserView *, double x, double y, double dx, double dy,
               uint32_t mods);
void bw_key(BrowserView *, const char *key, uint32_t mods, int down);
void bw_text(BrowserView *, const char *text, int preedit);
void bw_set_clipboard(BrowserView *, const char *text);
char *bw_take_clipboard(BrowserView *);
const char *bw_cursor(BrowserView *);
void bw_evaluate(BrowserView *, const char *script);
char *bw_take_evaluation(BrowserView *);
int bw_shutdown(void);
