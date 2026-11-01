#pragma once
#include <stddef.h>
#include <stdint.h>
typedef struct BrowserView BrowserView;
typedef struct {
  int javascript;
  int restricted_resources;
  int mediate_navigation;
  int theme;
  double background[4];
} BwConfig;
typedef struct {
  int fd;
  short events;
  short revents;
} BwPollFd;
typedef struct {
  uint64_t geometry_generation, received_frames, submitted_frames,
      discarded_frames;
  uint64_t import_hits, import_misses, cached_imports, outstanding_frames;
  int64_t geometry_us, frame_received_us, acquire_ready_us, submitted_us;
  uint64_t submitted_generation;
  int width, height;
} BwMetrics;
typedef struct {
  const char *key;
  const unsigned char *bytes;
  size_t length;
  const char *mime;
} BwResource;
typedef struct {
  unsigned flags, editing_flags;
  int x, y;
  const char *link_uri, *image_uri, *media_uri, *link_title;
} BwHitTest;
typedef struct {
  int kind, code;
  char *message, *uri;
} BwFailure;
int bw_hit_test(BrowserView *, BwHitTest *);
int bw_take_context_menu(BrowserView *, BwHitTest *);
unsigned bw_editing_state(BrowserView *);
int bw_clipboard_available(BrowserView *);
int bw_selection_query_supported(BrowserView *);
int bw_take_failure(BrowserView *, BwFailure *);
void bw_terminate_process(BrowserView *);
typedef struct {
  int phase;
  char *uri;
} BwLoadEvent;
int bw_take_load_event(BrowserView *, BwLoadEvent *);
int bw_request_selection_text(BrowserView *);
char *bw_take_selection_text(BrowserView *, int *status);
BrowserView *bw_create(void *, void *, char **);
BrowserView *bw_create_config(void *, void *, const BwConfig *, char **);
void bw_destroy(BrowserView *);
void bw_free(void *);
void bw_pump(void);
int bw_poll_prepare(BwPollFd *, int capacity, int *timeout_ms);
void bw_wake(void);
void bw_geometry(BrowserView *, int, int, double, int);
void bw_clip(BrowserView *, double, double, double, double);
void bw_focus(BrowserView *, int);
void bw_load(BrowserView *, const char *);
int bw_load_document(BrowserView *, uint64_t, const char *);
int bw_load_document_resources(BrowserView *, uint64_t, const char *,
                               const BwResource *, size_t);
int bw_set_resource(BrowserView *, uint64_t, const char *,
                    const unsigned char *, size_t, const char *);
void bw_fail_resource(BrowserView *, uint64_t, const char *);
char *bw_take_resource_request(BrowserView *);
void bw_action(BrowserView *, int);
void bw_zoom(BrowserView *, double);
double bw_get_zoom(BrowserView *);
const char *bw_uri(BrowserView *);
const char *bw_title(BrowserView *);
const char *bw_error(BrowserView *);
char *bw_take_new_uri(BrowserView *);
char *bw_take_navigation(BrowserView *, unsigned *flags, uint64_t *token);
void bw_resolve_navigation(BrowserView *, uint64_t token, int allow);
int bw_state(BrowserView *);
uint64_t bw_frames(BrowserView *);
uint64_t bw_document_id(BrowserView *);
int bw_sample_pixel(BrowserView *, int, int, unsigned char *);
int64_t bw_monotonic_us(void);
void bw_metrics(BrowserView *, BwMetrics *);
typedef struct {
  uint64_t views, displays, imports, pending_frames, sources, resource_waits;
  uint64_t document_bytes, resource_bytes;
  uint64_t policy_jobs, policy_waiters;
} BwLiveMetrics;
void bw_live_metrics(BwLiveMetrics *);
void bw_find(BrowserView *, const char *, int);
void bw_find_next(BrowserView *, int);
void bw_find_clear(BrowserView *);
unsigned bw_find_matches(BrowserView *);
int bw_find_pending(BrowserView *);
char *bw_runtime_info(void);
void bw_pointer(BrowserView *, int, double, double, int, uint32_t, int);
void bw_scroll(BrowserView *, double, double, double, double, uint32_t);
void bw_key(BrowserView *, const char *, uint32_t, int);
void bw_text(BrowserView *, const char *, int);
void bw_set_clipboard(BrowserView *, const char *);
char *bw_take_clipboard(BrowserView *);
const char *bw_cursor(BrowserView *);
void bw_evaluate(BrowserView *, const char *);
char *bw_take_evaluation(BrowserView *);
int bw_shutdown(void);
