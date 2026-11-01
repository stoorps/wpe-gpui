#define _GNU_SOURCE
#include "bridge.h"
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>
#include <dlfcn.h>
#include <drm_fourcc.h>
#include <glib-unix.h>
#include <glib/gstdio.h>
#include <inttypes.h>
#include <math.h>
#include <poll.h>
#include <stdarg.h>
#include <string.h>
#include <unistd.h>
#include <wayland-egl.h>
#include <wpe/webkit.h>
#include <wpe/wpe-platform.h>
#include <xkbcommon/xkbcommon.h>

/* All objects and callbacks belong to the GPUI UI thread. The Wayland objects
 * are borrowed; no connection dispatch or surface role is created here. */
typedef struct {
  WPEDisplay parent;
  BrowserView *owner;
  EGLDisplay egl;
  gboolean egl_handed_out;
  WPEDRMDevice *device;
  WPEBufferFormats *formats;
  WPEClipboard *clipboard;
  BrowserView *selection_owner;
  guint selection_timeout, deferred_source;
  gboolean clipboard_poisoned, setting_clipboard;
  GQueue copies_pending;
  GQueue deferred;
} DemoDisplay;
typedef WPEDisplayClass DemoDisplayClass;
typedef struct {
  DemoDisplay *display;
  BrowserView *owner;
  guint timeout;
} CopyWait;
typedef struct {
  WPEClipboard parent;
} BridgeClipboard;
typedef WPEClipboardClass BridgeClipboardClass;
typedef struct {
  BrowserView *owner;
  int kind, action;
  char *text;
  unsigned mods;
  int down;
} DeferredInput;
typedef struct {
  WPEToplevel parent;
} DemoToplevel;
typedef WPEToplevelClass DemoToplevelClass;
typedef struct {
  WPEView parent;
  BrowserView *owner;
} DemoView;
typedef WPEViewClass DemoViewClass;
typedef struct {
  WPEInputMethodContext parent;
  char *preedit;
} DemoIM;
typedef WPEInputMethodContextClass DemoIMClass;
G_DEFINE_TYPE(BridgeClipboard, bridge_clipboard, WPE_TYPE_CLIPBOARD)
G_DEFINE_TYPE(DemoDisplay, demo_display, WPE_TYPE_DISPLAY)
G_DEFINE_TYPE(DemoToplevel, demo_toplevel, WPE_TYPE_TOPLEVEL)
G_DEFINE_TYPE(DemoView, demo_view, WPE_TYPE_VIEW)
G_DEFINE_TYPE(DemoIM, demo_im, WPE_TYPE_INPUT_METHOD_CONTEXT)

/* WPEDisplay owns an isolated device/surfaceless EGLDisplay. Presenters
 * borrow GPUI's Wayland EGLDisplay and never terminate it. Share the engine
 * across tabs, then release it after explicit native close in the quit hook. */
static DemoDisplay *shared_displays[1];
static gboolean clipboard_scripts_used;
static unsigned live_views;
static GPtrArray *live_registry;

/* One constant transport blocker is compiled per engine. Async callbacks own
 * this job, never a BrowserView; closed views are removed from live_registry. */
typedef struct {
  unsigned refs;
  uint64_t generation;
  WebKitUserContentFilterStore *store;
  WebKitUserContentFilter *filter;
  GCancellable *cancel;
  char *directory, *error;
  guint timeout;
  gboolean pending, retired;
} ResourcePolicyJob;
static ResourcePolicyJob *resource_policy;
static uint64_t resource_policy_generation;
static unsigned resource_policy_jobs;

typedef struct {
  WPEBuffer *buffer;
  GLuint texture;
  EGLImage image;
  unsigned users;
  uint64_t used;
} BufferImport;
typedef struct {
  BrowserView *owner;
  WPEBuffer *buffer;
  BufferImport *import;
  EGLSync sync;
  int acquire_fd;
  guint source, timeout;
  gboolean rendered;
  uint64_t generation;
  int64_t received_us, acquire_us;
  uint64_t document_id;
} Frame;
typedef struct {
  GBytes *bytes;
  char *mime;
} Resource;
typedef struct {
  BrowserView *owner;
  WebKitURISchemeRequest *request;
  char *key;
  guint timeout;
} ResourceWait;

typedef struct {
  char *uri;
  unsigned flags;
  uint64_t token;
} NavigationEntry;
static void navigation_free(gpointer ptr) {
  NavigationEntry *entry = ptr;
  g_free(entry->uri);
  g_free(entry);
}
static void navigation_deny(gpointer ptr) {
  webkit_policy_decision_ignore(WEBKIT_POLICY_DECISION(ptr));
  g_object_unref(ptr);
}

struct BrowserView {
  DemoDisplay *display;
  WebKitWebContext *context;
  WebKitNetworkSession *session;
  WebKitWebView *web;
  WPEView *view; /* Borrowed from web. */
  WPEToplevel *toplevel;
  DemoIM *im; /* Borrowed from WebKit. */
  struct wl_egl_window *window;
  EGLDisplay egl; /* Borrowed Wayland EGL display; never terminated here. */
  EGLSurface surface;
  EGLContext gl;
  GLuint program, texture;
  GLuint snapshot_texture, snapshot_fbo;
  int snapshot_width, snapshot_height;
  uint64_t snapshot_document_id;
  gboolean snapshot_valid;
  double clip_x, clip_y, clip_width, clip_height;
  gboolean clip_set;
  int width, height, visible, restricted, javascript, registered;
  gboolean mediate_navigation;
  WebKitUserContentManager *content_manager;
  uint64_t policy_generation;
  gboolean policy_ready, policy_failed, policy_load_queued;
  char *selection_result;
  int selection_status;
  char *approved_uri;
  uint64_t navigation_token;
  GHashTable *navigation_pending;
  WebKitHitTestResult *hit, *context_hit;
  gboolean context_requested;
  int pointer_x, pointer_y, context_x, context_y;
  GQueue failures, load_events;
  gboolean awaiting_commit, renderer_failed;
  uint64_t document_id, pending_document_id;
  char *document_html, *document_uri;
  GHashTable *resources;
  GHashTable *documents;
  GQueue document_order;
  GHashTable *resource_waits;
  GQueue resource_events, navigation_events;
  GQueue frames_pending;
  GPtrArray *imports;
  guint completion_source;
  uint64_t import_clock;
  BwMetrics metrics;
  unsigned find_matches;
  gboolean find_pending;
  WebKitColor background;
  double scale;
  uint64_t frames;
  char *error, *cursor;
  gint64 clipboard_count;
};

static void trace_native(const char *format, ...) {
  static int enabled = -1;
  if (enabled < 0)
    enabled = g_getenv("GPUI_WPE_TRACE") != NULL;
  if (!enabled)
    return;
  va_list args;
  va_start(args, format);
  char *message = g_strdup_vprintf(format, args);
  va_end(args);
  g_printerr("WPE_TRACE %" G_GINT64_FORMAT " %s\n", g_get_monotonic_time(),
             message);
  g_free(message);
}
static void failure_free(gpointer ptr) {
  BwFailure *failure = ptr;
  g_free(failure->message);
  g_free(failure->uri);
  g_free(failure);
}
static void record_failure(BrowserView *b, int kind, int code,
                           const char *message, const char *uri) {
  if (g_queue_get_length(&b->failures) >= 32)
    failure_free(g_queue_pop_head(&b->failures));
  BwFailure *failure = g_new0(BwFailure, 1);
  failure->kind = kind;
  failure->code = code;
  failure->message = g_strdup(message);
  failure->uri = g_strdup(uri);
  g_queue_push_tail(&b->failures, failure);
  g_free(b->error);
  b->error = g_strdup(message);
  bw_wake();
}
static void policy_unref(ResourcePolicyJob *job) {
  if (--job->refs)
    return;
  if (job->timeout)
    g_source_remove(job->timeout);
  if (job->filter)
    webkit_user_content_filter_unref(job->filter);
  g_clear_object(&job->store);
  g_clear_object(&job->cancel);
  /* Only this process creates files in its private 0700 directory. Cleanup is
   * after the save callback, hence never races the compiler's file writes. */
  if (job->directory) {
    GDir *directory = g_dir_open(job->directory, 0, NULL);
    if (directory) {
      const char *name;
      while ((name = g_dir_read_name(directory))) {
        char *path = g_build_filename(job->directory, name, NULL);
        g_unlink(path);
        g_free(path);
      }
      g_dir_close(directory);
    }
    g_rmdir(job->directory);
  }
  g_free(job->directory);
  g_free(job->error);
  g_free(job);
  resource_policy_jobs--;
}
static void policy_notify(ResourcePolicyJob *job) {
  if (!job || job->retired || resource_policy != job || !live_registry)
    return;
  for (guint i = 0; i < live_registry->len; i++) {
    BrowserView *b = g_ptr_array_index(live_registry, i);
    if (!b->restricted || b->policy_generation != job->generation ||
        b->policy_failed || b->policy_ready)
      continue;
    if (job->filter) {
      webkit_user_content_manager_add_filter(b->content_manager, job->filter);
      b->policy_ready = TRUE;
      if (b->policy_load_queued) {
        b->policy_load_queued = FALSE;
        webkit_web_view_load_uri(b->web, b->document_uri);
      }
    } else if (job->error) {
      b->policy_failed = TRUE;
      b->policy_load_queued = FALSE;
      record_failure(b, 1, 0, job->error, b->document_uri);
    }
  }
  bw_wake();
}
static gboolean policy_timeout(gpointer data) {
  ResourcePolicyJob *job = data;
  job->timeout = 0;
  if (job->pending && !job->error) {
    job->error = g_strdup("Provided-resource transport filter compilation timed out");
    g_cancellable_cancel(job->cancel);
    policy_notify(job);
  }
  return G_SOURCE_REMOVE;
}
static void policy_compiled(GObject *source, GAsyncResult *result, gpointer data) {
  ResourcePolicyJob *job = data;
  GError *error = NULL;
  WebKitUserContentFilter *filter = webkit_user_content_filter_store_save_finish(
      WEBKIT_USER_CONTENT_FILTER_STORE(source), result, &error);
  job->pending = FALSE;
  if (job->timeout) {
    g_source_remove(job->timeout);
    job->timeout = 0;
  }
  if (!job->retired && !job->error) {
    if (filter) {
      job->filter = filter;
      filter = NULL;
    } else {
      job->error = g_strdup_printf("Provided-resource transport filter failed: %s",
                                   error ? error->message : "unknown error");
    }
    policy_notify(job);
  }
  if (filter)
    webkit_user_content_filter_unref(filter);
  g_clear_error(&error);
  policy_unref(job);
}
static ResourcePolicyJob *policy_ensure(void) {
  if (resource_policy)
    return resource_policy;
  ResourcePolicyJob *job = g_new0(ResourcePolicyJob, 1);
  resource_policy_jobs++;
  job->refs = 1; /* Engine ownership, independent of the async operation. */
  job->generation = ++resource_policy_generation;
  job->cancel = g_cancellable_new();
  resource_policy = job;
  GError *error = NULL;
  job->directory = g_dir_make_tmp("gpui-wpe-policy-XXXXXX", &error);
  if (!job->directory) {
    job->error = g_strdup_printf("Cannot create private resource-policy store: %s",
                                 error->message);
    g_clear_error(&error);
    return job;
  }
  job->store = webkit_user_content_filter_store_new(job->directory);
  static const char rules[] =
      "[{\"trigger\":{\"url-filter\":\"^https?://\"},"
      "\"action\":{\"type\":\"block\"}}]";
  const char *source = rules;
#ifdef BW_NATIVE_FAILURE_INJECTION
  /* Opt-in process diagnostics exercise the real asynchronous compiler error
   * path, without creating a weaker runtime policy in ordinary builds. */
  if (!g_strcmp0(g_getenv("GPUI_WPE_POLICY_FAULT"), "compile"))
    source = "invalid content-filter JSON";
#endif
  GBytes *bytes = g_bytes_new_static(source, strlen(source));
  job->pending = TRUE;
  job->timeout = g_timeout_add(5000, policy_timeout, job);
  job->refs++;
  webkit_user_content_filter_store_save(job->store, "provided-resources", bytes,
                                        job->cancel, policy_compiled, job);
  g_bytes_unref(bytes);
  return job;
}
static void set_error(BrowserView *b, const char *message) {
  record_failure(b, 3, 0, message,
                 b->web ? webkit_web_view_get_uri(b->web) : NULL);
}
static void copy_wait_free(CopyWait *wait) {
  if (wait->timeout)
    g_source_remove(wait->timeout);
  g_free(wait);
}
static void clipboard_poison(DemoDisplay *d, const char *message) {
  d->clipboard_poisoned = TRUE;
  while (!g_queue_is_empty(&d->copies_pending)) {
    CopyWait *wait = g_queue_pop_head(&d->copies_pending);
    copy_wait_free(wait);
  }
  if (live_registry)
    for (guint i = 0; i < live_registry->len; i++) {
      BrowserView *b = g_ptr_array_index(live_registry, i);
      if (b->display == d)
        record_failure(b, 4, 0, message, webkit_web_view_get_uri(b->web));
    }
}
int bw_clipboard_available(BrowserView *b) {
  return !b->display->clipboard_poisoned;
}
int bw_selection_query_supported(BrowserView *b) {
  return !b->javascript && !clipboard_scripts_used && bw_clipboard_available(b);
}
static gboolean copy_timeout(gpointer data) {
  CopyWait *wait = data;
  wait->timeout = 0;
  clipboard_poison(
      wait->display,
      "Clipboard Copy timed out; recreate engine to recover clipboard");
  return G_SOURCE_REMOVE;
}
static gboolean copy_started(BrowserView *b) {
  DemoDisplay *d = b->display;
  if (g_queue_get_length(&d->copies_pending) >= 64) {
    set_error(b, "Clipboard Copy queue limit reached");
    return FALSE;
  }
  CopyWait *wait = g_new0(CopyWait, 1);
  wait->display = d;
  wait->owner = b;
  wait->timeout = g_timeout_add_seconds(2, copy_timeout, wait);
  g_queue_push_tail(&d->copies_pending, wait);
  return TRUE;
}
static void cancel_owned_copies(BrowserView *b) {
  // A normal user Copy is still intended after its tab closes. Keep its FIFO
  // slot until reply/deadline, detach only the dying owner, and keep queries
  // busy. Successful late user copies therefore do not poison the pool.
  for (GList *link = b->display->copies_pending.head; link; link = link->next) {
    CopyWait *wait = link->data;
    if (wait->owner == b)
      wait->owner = NULL;
  }
}
static void deferred_free(gpointer ptr) {
  DeferredInput *input = ptr;
  g_free(input->text);
  g_free(input);
}
static gboolean deferred_dispatch(gpointer data) {
  DemoDisplay *d = data;
  d->deferred_source = 0;
  while (!g_queue_is_empty(&d->deferred)) {
    DeferredInput *input = g_queue_pop_head(&d->deferred);
    if (d->clipboard_poisoned)
      set_error(input->owner,
                "Clipboard query failed; recreate engine to recover clipboard");
    else if (input->kind == 0)
      bw_action(input->owner, input->action);
    else if (input->kind == 1)
      bw_key(input->owner, input->text, input->mods, input->down);
    else
      bw_set_clipboard(input->owner, input->text);
    deferred_free(input);
  }
  return G_SOURCE_REMOVE;
}
static void flush_deferred(DemoDisplay *d) {
  if (!g_queue_is_empty(&d->deferred) && !d->deferred_source)
    d->deferred_source = g_idle_add_full(G_PRIORITY_DEFAULT, deferred_dispatch,
                                         g_object_ref(d), g_object_unref);
}
static gboolean defer_input(BrowserView *b, int kind, int action,
                            const char *text, unsigned mods, int down) {
  DemoDisplay *d = b->display;
  if (d->clipboard_poisoned) {
    set_error(b,
              "Clipboard query failed; recreate engine to recover clipboard");
    return TRUE;
  }
  if (!d->selection_owner)
    return FALSE;
  if (g_queue_get_length(&d->deferred) >= 64) {
    set_error(b, "Clipboard input queue limit reached");
    return TRUE;
  }
  DeferredInput *input = g_new0(DeferredInput, 1);
  *input = (DeferredInput){b, kind, action, g_strdup(text), mods, down};
  g_queue_push_tail(&d->deferred, input);
  return TRUE;
}
static void selection_cancel(BrowserView *b, const char *message) {
  DemoDisplay *d = b->display;
  if (!d || d->selection_owner != b)
    return;
  if (d->selection_timeout)
    g_source_remove(d->selection_timeout);
  d->selection_timeout = 0;
  d->selection_owner = NULL;
  // No correlation token accompanies a delayed WebKit Copy reply. Fail closed
  // after cancellation so it can never become a user/system clipboard write.
  clipboard_poison(d, message);
  g_free(b->selection_result);
  b->selection_result = g_strdup(message);
  b->selection_status = -1;
  flush_deferred(d);
  bw_wake();
}
static gboolean selection_timeout(gpointer data) {
  DemoDisplay *d = data;
  d->selection_timeout = 0;
  if (d->selection_owner)
    selection_cancel(d->selection_owner, "Selection query timed out");
  return G_SOURCE_REMOVE;
}
static void clipboard_changed(WPEClipboard *clipboard, GPtrArray *formats,
                              gboolean local, WPEClipboardContent *content) {
  DemoDisplay *d = (DemoDisplay *)wpe_clipboard_get_display(clipboard);
  if (d->selection_owner) {
    BrowserView *b = d->selection_owner;
    if (d->selection_timeout)
      g_source_remove(d->selection_timeout);
    d->selection_timeout = 0;
    d->selection_owner = NULL;
    const char *text = content ? wpe_clipboard_content_get_text(content) : NULL;
    g_free(b->selection_result);
    b->selection_result = g_strdup(text ? text : "");
    b->selection_status = 1;
    // Do not call parent changed(): query Copy never changes clipboard content
    // or its change count and therefore cannot be forwarded to the host.
    flush_deferred(d);
    bw_wake();
    return;
  }
  if (d->clipboard_poisoned)
    return;
  if (!d->setting_clipboard && !g_queue_is_empty(&d->copies_pending))
    copy_wait_free(g_queue_pop_head(&d->copies_pending));
  WPE_CLIPBOARD_CLASS(bridge_clipboard_parent_class)
      ->changed(clipboard, formats, local, content);
  bw_wake();
}
static void bridge_clipboard_class_init(BridgeClipboardClass *c) {
  c->changed = clipboard_changed;
}
static void bridge_clipboard_init(BridgeClipboard *c) { (void)c; }
int bw_request_selection_text(BrowserView *b) {
  DemoDisplay *d = b->display;
  if (!bw_selection_query_supported(b))
    return -1;
  if (d->clipboard_poisoned || d->selection_owner ||
      !g_queue_is_empty(&d->copies_pending) || b->selection_status)
    return 0;
  if (!(bw_editing_state(b) & 1)) {
    b->selection_result = g_strdup("");
    b->selection_status = 1;
    bw_wake();
    return 1;
  }
  d->selection_owner = b;
  d->selection_timeout = g_timeout_add_seconds(2, selection_timeout, d);
  webkit_web_view_execute_editing_command(b->web, "Copy");
  bw_wake();
  return 1;
}
char *bw_take_selection_text(BrowserView *b, int *status) {
  *status = b->selection_status;
  if (!b->selection_status)
    return NULL;
  b->selection_status = 0;
  return g_steal_pointer(&b->selection_result);
}
static uint32_t event_time(void) {
  return (uint32_t)(g_get_monotonic_time() / 1000);
}
static gboolean display_connect(WPEDisplay *d, GError **e) {
  (void)d;
  (void)e;
  return TRUE;
}
static WPEView *display_view(WPEDisplay *d) {
  DemoView *v = g_object_new(demo_view_get_type(), "display", d, NULL);
  v->owner = ((DemoDisplay *)d)->owner;
  return WPE_VIEW(v);
}
static WPEToplevel *display_toplevel(WPEDisplay *d, guint count) {
  return g_object_new(demo_toplevel_get_type(), "display", d, "max-views",
                      count, NULL);
}
static gpointer display_egl(WPEDisplay *d, GError **e) {
  (void)e;
  ((DemoDisplay *)d)->egl_handed_out = TRUE;
  return ((DemoDisplay *)d)->egl;
}
static WPEDRMDevice *display_device(WPEDisplay *d) {
  return ((DemoDisplay *)d)->device;
}
static WPEBufferFormats *display_formats(WPEDisplay *d) {
  return g_object_ref(((DemoDisplay *)d)->formats);
}
static WPEClipboard *display_clipboard(WPEDisplay *d) {
  DemoDisplay *self = (DemoDisplay *)d;
  if (!self->clipboard)
    self->clipboard =
        g_object_new(bridge_clipboard_get_type(), "display", d, NULL);
  return self->clipboard;
}
static WPEInputMethodContext *display_im(WPEDisplay *d, WPEView *v) {
  (void)d;
  DemoIM *im = g_object_new(demo_im_get_type(), "view", v, NULL);
  BrowserView *b = ((DemoView *)v)->owner;
  if (b) {
    if (b->im)
      g_object_remove_weak_pointer(G_OBJECT(b->im), (gpointer *)&b->im);
    b->im = im;
    g_object_add_weak_pointer(G_OBJECT(im), (gpointer *)&b->im);
  }
  return WPE_INPUT_METHOD_CONTEXT(im);
}
static void display_dispose(GObject *o) {
  DemoDisplay *d = (DemoDisplay *)o;
  g_clear_object(&d->formats);
  g_clear_object(&d->clipboard);
  if (d->device) {
    wpe_drm_device_unref(d->device);
    d->device = NULL;
  }
  // Before handing the owned device display to WPE, an initialization error
  // leaves it outside the base class's cached EGL ownership. Clean that path.
  if (!d->egl_handed_out && d->egl != EGL_NO_DISPLAY) {
    eglTerminate(d->egl);
    d->egl = EGL_NO_DISPLAY;
  }
  G_OBJECT_CLASS(demo_display_parent_class)->dispose(o);
}
static void demo_display_class_init(DemoDisplayClass *c) {
  c->connect = display_connect;
  c->create_view = display_view;
  c->create_toplevel = display_toplevel;
  c->get_egl_display = display_egl;
  c->get_drm_device = display_device;
  c->get_preferred_buffer_formats = display_formats;
  c->get_clipboard = display_clipboard;
  c->create_input_method_context = display_im;
  G_OBJECT_CLASS(c)->dispose = display_dispose;
}
static void demo_display_init(DemoDisplay *d) { d->egl = EGL_NO_DISPLAY; }
static void demo_toplevel_class_init(DemoToplevelClass *c) { (void)c; }
static void demo_toplevel_init(DemoToplevel *t) { (void)t; }
static void im_preedit(WPEInputMethodContext *ctx, char **text, GList **lines,
                       guint *cursor) {
  const char *s = ((DemoIM *)ctx)->preedit;
  *text = g_strdup(s ? s : "");
  *lines = NULL;
  *cursor = g_utf8_strlen(*text, -1);
}
static void im_finalize(GObject *o) {
  g_free(((DemoIM *)o)->preedit);
  G_OBJECT_CLASS(demo_im_parent_class)->finalize(o);
}
static void demo_im_class_init(DemoIMClass *c) {
  c->get_preedit_string = im_preedit;
  G_OBJECT_CLASS(c)->finalize = im_finalize;
}
static void demo_im_init(DemoIM *im) { im->preedit = NULL; }

static GLuint shader(GLenum type, const char *source) {
  GLuint s = glCreateShader(type);
  glShaderSource(s, 1, &source, NULL);
  glCompileShader(s);
  GLint ok;
  glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
  if (!ok) {
    glDeleteShader(s);
    return 0;
  }
  return s;
}
static void import_free(BrowserView *b, BufferImport *entry) {
  if (entry->image != EGL_NO_IMAGE_KHR) {
    PFNEGLDESTROYIMAGEKHRPROC destroy =
        (void *)eglGetProcAddress("eglDestroyImageKHR");
    if (destroy)
      destroy(b->egl, entry->image);
  }
  if (entry->texture)
    glDeleteTextures(1, &entry->texture);
  g_object_unref(entry->buffer);
  g_free(entry);
}
static BufferImport *buffer_import(BrowserView *b, WPEBuffer *buffer,
                                   GError **error) {
  (void)error;
  BufferImport *entry = NULL;
  for (guint i = 0; i < b->imports->len; i++) {
    BufferImport *candidate = g_ptr_array_index(b->imports, i);
    if (candidate->buffer == buffer) {
      entry = candidate;
      b->metrics.import_hits++;
      break;
    }
  }
  if (entry && !WPE_IS_BUFFER_SHM(buffer)) {
    entry->used = ++b->import_clock;
    return entry;
  }
  if (!entry) {
    if (b->imports->len >= 8) {
      guint victim = G_MAXUINT;
      uint64_t oldest = UINT64_MAX;
      for (guint i = 0; i < b->imports->len; i++) {
        BufferImport *e = g_ptr_array_index(b->imports, i);
        if (!e->users && e->used < oldest) {
          victim = i;
          oldest = e->used;
        }
      }
      if (victim == G_MAXUINT)
        return NULL;
      import_free(b, g_ptr_array_index(b->imports, victim));
      g_ptr_array_remove_index_fast(b->imports, victim);
    }
    entry = g_new0(BufferImport, 1);
    entry->buffer = g_object_ref(buffer);
    entry->image = EGL_NO_IMAGE_KHR;
    glGenTextures(1, &entry->texture);
    glBindTexture(GL_TEXTURE_2D, entry->texture);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    g_ptr_array_add(b->imports, entry);
    b->metrics.import_misses++;
  } else
    glBindTexture(GL_TEXTURE_2D, entry->texture);
  int width = wpe_buffer_get_width(buffer),
      height = wpe_buffer_get_height(buffer);
  if (WPE_IS_BUFFER_DMA_BUF(buffer)) {
    WPEBufferDMABuf *dmabuf = WPE_BUFFER_DMA_BUF(buffer);
    guint n_planes = wpe_buffer_dma_buf_get_n_planes(dmabuf);
    if (!n_planes || n_planes > 4)
      goto failed;
    EGLint attrs[64];
    int a = 0;
    attrs[a++] = EGL_WIDTH;
    attrs[a++] = width;
    attrs[a++] = EGL_HEIGHT;
    attrs[a++] = height;
    attrs[a++] = EGL_LINUX_DRM_FOURCC_EXT;
    attrs[a++] = wpe_buffer_dma_buf_get_format(dmabuf);
    const EGLint fd_keys[] = {
        EGL_DMA_BUF_PLANE0_FD_EXT, EGL_DMA_BUF_PLANE1_FD_EXT,
        EGL_DMA_BUF_PLANE2_FD_EXT, EGL_DMA_BUF_PLANE3_FD_EXT};
    const EGLint offset_keys[] = {
        EGL_DMA_BUF_PLANE0_OFFSET_EXT, EGL_DMA_BUF_PLANE1_OFFSET_EXT,
        EGL_DMA_BUF_PLANE2_OFFSET_EXT, EGL_DMA_BUF_PLANE3_OFFSET_EXT};
    const EGLint pitch_keys[] = {
        EGL_DMA_BUF_PLANE0_PITCH_EXT, EGL_DMA_BUF_PLANE1_PITCH_EXT,
        EGL_DMA_BUF_PLANE2_PITCH_EXT, EGL_DMA_BUF_PLANE3_PITCH_EXT};
    const EGLint lo_keys[] = {
        EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT, EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT,
        EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT, EGL_DMA_BUF_PLANE3_MODIFIER_LO_EXT};
    const EGLint hi_keys[] = {
        EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT, EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT,
        EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT, EGL_DMA_BUF_PLANE3_MODIFIER_HI_EXT};
    guint64 modifier = wpe_buffer_dma_buf_get_modifier(dmabuf);
    for (guint plane = 0; plane < n_planes; plane++) {
      attrs[a++] = fd_keys[plane];
      attrs[a++] = wpe_buffer_dma_buf_get_fd(dmabuf, plane);
      attrs[a++] = offset_keys[plane];
      attrs[a++] = wpe_buffer_dma_buf_get_offset(dmabuf, plane);
      attrs[a++] = pitch_keys[plane];
      attrs[a++] = wpe_buffer_dma_buf_get_stride(dmabuf, plane);
      if (modifier != DRM_FORMAT_MOD_INVALID) {
        attrs[a++] = lo_keys[plane];
        attrs[a++] = (EGLint)(modifier & 0xffffffff);
        attrs[a++] = hi_keys[plane];
        attrs[a++] = (EGLint)(modifier >> 32);
      }
    }
    attrs[a] = EGL_NONE;
    PFNEGLCREATEIMAGEKHRPROC create_image =
        (void *)eglGetProcAddress("eglCreateImageKHR");
    PFNEGLDESTROYIMAGEKHRPROC destroy_image =
        (void *)eglGetProcAddress("eglDestroyImageKHR");
    PFNGLEGLIMAGETARGETTEXTURE2DOESPROC bind_image =
        (void *)eglGetProcAddress("glEGLImageTargetTexture2DOES");
    if (!create_image || !destroy_image || !bind_image)
      goto failed;
    EGLImage image = create_image(b->egl, EGL_NO_CONTEXT, EGL_LINUX_DMA_BUF_EXT,
                                  NULL, attrs);
    if (image == EGL_NO_IMAGE_KHR)
      goto failed;
    bind_image(GL_TEXTURE_2D, image);
    entry->image = image;
  } else if (WPE_IS_BUFFER_SHM(buffer)) {
    WPEBufferSHM *shm = WPE_BUFFER_SHM(buffer);
    if (wpe_buffer_shm_get_format(shm) != WPE_PIXEL_FORMAT_ARGB8888)
      goto failed;
    GBytes *bytes = wpe_buffer_shm_get_data(shm);
    gsize length = 0;
    const guint8 *src = g_bytes_get_data(bytes, &length);
    int stride = wpe_buffer_shm_get_stride(shm);
    if (width <= 0 || height <= 0 || width > 16384 || height > 16384 ||
        stride < width * 4 || (gsize)stride * height > length)
      goto failed;
    guint8 *rgba = g_malloc((gsize)width * height * 4);
    for (int y = 0; y < height; y++)
      for (int x = 0; x < width; x++) {
        const guint8 *p = src + y * stride + x * 4;
        guint8 *q = rgba + (y * width + x) * 4;
        q[0] = p[2];
        q[1] = p[1];
        q[2] = p[0];
        q[3] = p[3];
      }
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, width, height, 0, GL_RGBA,
                 GL_UNSIGNED_BYTE, rgba);
    g_free(rgba);
  } else
    goto failed;

  if (glGetError() != GL_NO_ERROR)
    goto failed;
  entry->used = ++b->import_clock;
  b->metrics.cached_imports = b->imports->len;
  return entry;
failed:
  for (guint i = 0; i < b->imports->len; i++)
    if (g_ptr_array_index(b->imports, i) == entry) {
      g_ptr_array_remove_index_fast(b->imports, i);
      break;
    }
  import_free(b, entry);
  b->metrics.cached_imports = b->imports->len;
  return NULL;
}
static void frame_release(Frame *frame) {
  BrowserView *b = frame->owner;
  if (frame->source)
    g_source_remove(frame->source);
  if (frame->timeout)
    g_source_remove(frame->timeout);
  if (frame->acquire_fd >= 0)
    close(frame->acquire_fd);
  if (frame->sync != EGL_NO_SYNC_KHR) {
    PFNEGLDESTROYSYNCKHRPROC destroy =
        (void *)eglGetProcAddress("eglDestroySyncKHR");
    if (destroy)
      destroy(b->egl, frame->sync);
  }
  if (frame->import)
    frame->import->users--;
  if (!frame->rendered) {
    wpe_view_buffer_rendered(b->view, frame->buffer);
    b->metrics.discarded_frames++;
  }
  wpe_view_buffer_released(b->view, frame->buffer);
  g_object_unref(frame->buffer);
  g_queue_remove(&b->frames_pending, frame);
  g_free(frame);
  b->metrics.outstanding_frames = g_queue_get_length(&b->frames_pending);
}
static gboolean complete_frames(gpointer data) {
  BrowserView *b = data;
  if (!eglMakeCurrent(b->egl, b->surface, b->surface, b->gl)) {
    set_error(b, "Cannot restore EGL context while completing frames");
    return G_SOURCE_CONTINUE;
  }
  PFNEGLCLIENTWAITSYNCKHRPROC wait =
      (void *)eglGetProcAddress("eglClientWaitSyncKHR");
  gboolean gpu_pending = FALSE;
  GList *link = b->frames_pending.head;
  while (link) {
    Frame *frame = link->data;
    link = link->next;
    if (frame->sync != EGL_NO_SYNC_KHR) {
      EGLint state = wait(b->egl, frame->sync, 0, 0);
      if (state == EGL_CONDITION_SATISFIED_KHR)
        frame_release(frame);
      else {
        gpu_pending = TRUE;
        if (state == EGL_FALSE)
          set_error(b, "GPU completion fence failed");
      }
    }
  }
  if (!gpu_pending) {
    b->completion_source = 0;
    return G_SOURCE_REMOVE;
  }
  return G_SOURCE_CONTINUE;
}
static void draw_texture(BrowserView *b, GLuint texture, int width, int height,
                         const GLfloat *vertices) {
  glBindTexture(GL_TEXTURE_2D, texture);
  glViewport(0, 0, width, height);
  glUseProgram(b->program);
  glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 4 * sizeof(GLfloat),
                        vertices);
  glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 4 * sizeof(GLfloat),
                        vertices + 2);
  glEnableVertexAttribArray(0);
  glEnableVertexAttribArray(1);
  glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
}
static gboolean snapshot_prepare(BrowserView *b, int width, int height) {
  if (width <= 0 || height <= 0 || width > 16384 || height > 16384 ||
      (uint64_t)width * height > 16777216)
    return FALSE;
  if (b->snapshot_width == width && b->snapshot_height == height &&
      b->snapshot_texture)
    return TRUE;
  b->snapshot_valid = FALSE;
  if (b->snapshot_texture)
    glDeleteTextures(1, &b->snapshot_texture);
  if (!b->snapshot_fbo)
    glGenFramebuffers(1, &b->snapshot_fbo);
  glGenTextures(1, &b->snapshot_texture);
  glBindTexture(GL_TEXTURE_2D, b->snapshot_texture);
  glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, width, height, 0, GL_RGBA,
               GL_UNSIGNED_BYTE, NULL);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
  glBindFramebuffer(GL_FRAMEBUFFER, b->snapshot_fbo);
  glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D,
                         b->snapshot_texture, 0);
  gboolean complete =
      glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE;
  glBindFramebuffer(GL_FRAMEBUFFER, 0);
  if (!complete)
    return FALSE;
  b->snapshot_width = width;
  b->snapshot_height = height;
  return TRUE;
}
static gboolean present_snapshot(BrowserView *b) {
  if (!b->visible || !b->snapshot_valid || !b->snapshot_texture ||
      b->snapshot_document_id != b->document_id)
    return FALSE;
  if (b->snapshot_width != (int)ceil(b->width * b->scale) ||
      b->snapshot_height != (int)ceil(b->height * b->scale))
    return FALSE;
  double x = b->clip_set ? b->clip_x : 0, y = b->clip_set ? b->clip_y : 0;
  double width = b->clip_set ? b->clip_width : b->width,
         height = b->clip_set ? b->clip_height : b->height;
  if (width <= 0 || height <= 0)
    return FALSE;
  GLfloat left = x / b->width, right = (x + width) / b->width;
  GLfloat top = 1 - y / b->height, bottom = 1 - (y + height) / b->height;
  const GLfloat vertices[] = {-1, -1, left, bottom, 1, -1, right, bottom,
                              -1, 1,  left, top,    1, 1,  right, top};
  glBindFramebuffer(GL_FRAMEBUFFER, 0);
  draw_texture(b, b->snapshot_texture, MAX(1, (int)ceil(width * b->scale)),
               MAX(1, (int)ceil(height * b->scale)), vertices);
  glFlush();
  return eglSwapBuffers(b->egl, b->surface);
}
static void frame_submit(Frame *frame) {
  BrowserView *b = frame->owner;
  if (frame->acquire_fd >= 0) {
    close(frame->acquire_fd);
    frame->acquire_fd = -1;
  }
  if (frame->timeout) {
    g_source_remove(frame->timeout);
    frame->timeout = 0;
  }
  frame->acquire_us = g_get_monotonic_time();

  int width = wpe_buffer_get_width(frame->buffer),
      height = wpe_buffer_get_height(frame->buffer);
  if (b->renderer_failed || (b->restricted && !b->policy_ready) ||
      frame->document_id != b->document_id ||
      b->awaiting_commit || !b->visible ||
      width != (int)ceil(b->width * b->scale) ||
      height != (int)ceil(b->height * b->scale)) {
    frame_release(frame);
    return;
  }
  if (!eglMakeCurrent(b->egl, b->surface, b->surface, b->gl)) {
    set_error(b, "Cannot make EGL context current");
    frame_release(frame);
    return;
  }
  GError *error = NULL;
  BufferImport *entry = buffer_import(b, frame->buffer, &error);
  if (!entry) {
    set_error(b,
              error ? error->message : "DMA-BUF import or import cache failed");
    g_clear_error(&error);
    frame_release(frame);
    return;
  }
  frame->import = entry;
  entry->users++;
  if (!snapshot_prepare(b, width, height)) {
    set_error(b, "Cannot allocate bounded viewport snapshot");
    frame_release(frame);
    return;
  }
  static const GLfloat vertices[] = {-1, -1, 0, 1, 1, -1, 1, 1,
                                     -1, 1,  0, 0, 1, 1,  1, 0};
  glBindFramebuffer(GL_FRAMEBUFFER, b->snapshot_fbo);
  draw_texture(b, entry->texture, width, height, vertices);
  glBindFramebuffer(GL_FRAMEBUFFER, 0);
  b->snapshot_document_id = b->document_id;
  b->snapshot_valid = TRUE;
  PFNEGLCREATESYNCKHRPROC create =
      (void *)eglGetProcAddress("eglCreateSyncKHR");
  frame->sync = create(b->egl, EGL_SYNC_FENCE_KHR, NULL);
  if (frame->sync == EGL_NO_SYNC_KHR) {
    // Exceptional driver/allocation failure only. Pause this renderer before
    // a synchronous completion fallback; the normal frame path never glFinish.
    b->renderer_failed = TRUE;
    for (int i = 0; i < 16 && glGetError() != GL_NO_ERROR; i++) {
    }
    glFinish();
    if (glGetError() != GL_NO_ERROR)
      g_error("Cannot establish GPU completion after fence failure; refusing "
              "unsafe buffer reuse");
    set_error(b, "GPU completion fence allocation failed; renderer paused. "
                 "Recreate the WebView to recover.");
    wpe_view_set_visible(b->view, FALSE);
    wpe_view_unmap(b->view);
    frame_release(frame);
    return;
  }
  glFlush();
  if (!present_snapshot(b)) {
    set_error(b, "EGL swap failed");
  } else {
    b->frames++;
    b->metrics.submitted_frames++;
    b->metrics.frame_received_us = frame->received_us;
    b->metrics.acquire_ready_us = frame->acquire_us;
    b->metrics.submitted_us = g_get_monotonic_time();
    b->metrics.submitted_generation = frame->generation;
  }
  // This is submission, not compositor-presentation feedback. Source release
  // remains independently delayed until the GPU fence signals.
  wpe_view_buffer_rendered(b->view, frame->buffer);
  frame->rendered = TRUE;
  if (!b->completion_source)
    b->completion_source =
        g_timeout_add_full(G_PRIORITY_DEFAULT, 1, complete_frames, b, NULL);
  bw_wake();
}
static gboolean acquired(int fd, GIOCondition condition, gpointer data) {
  (void)fd;
  Frame *frame = data;
  frame->source = 0;
  if (condition & G_IO_IN)
    frame_submit(frame);
  else {
    set_error(frame->owner, "Producer acquire fence failed");
    frame_release(frame);
  }
  return G_SOURCE_REMOVE;
}
static gboolean acquire_timeout(gpointer data) {
  Frame *frame = data;
  frame->timeout = 0;
  set_error(frame->owner, "Producer acquire fence timed out");
  frame_release(frame);
  return G_SOURCE_REMOVE;
}
static gboolean render_buffer(WPEView *view, WPEBuffer *buffer,
                              const WPERectangle *damage, guint count,
                              GError **error) {
  (void)damage;
  (void)count;
  BrowserView *b = ((DemoView *)view)->owner;
  if (!b) {
    g_set_error_literal(error, WPE_VIEW_ERROR, WPE_VIEW_ERROR_RENDER_FAILED,
                        "View is closing");
    return FALSE;
  }
  if (g_queue_get_length(&b->frames_pending) >= 4) {
    wpe_view_buffer_rendered(view, buffer);
    wpe_view_buffer_released(view, buffer);
    b->metrics.discarded_frames++;
    return TRUE;
  }
  Frame *frame = g_new0(Frame, 1);
  trace_native("frame view=%p size=%dx%d damage=%u rect=%d,%d %dx%d", b,
               wpe_buffer_get_width(buffer), wpe_buffer_get_height(buffer),
               count, count ? damage[0].x : 0, count ? damage[0].y : 0,
               count ? damage[0].width : 0, count ? damage[0].height : 0);
  frame->owner = b;
  frame->buffer = g_object_ref(buffer);
  frame->acquire_fd = wpe_buffer_take_rendering_fence(buffer);
  frame->sync = EGL_NO_SYNC_KHR;
  frame->received_us = g_get_monotonic_time();
  frame->generation = b->metrics.geometry_generation;
  frame->document_id = b->document_id;
  b->metrics.received_frames++;

  g_queue_push_tail(&b->frames_pending, frame);
  b->metrics.outstanding_frames = g_queue_get_length(&b->frames_pending);
  if (frame->acquire_fd >= 0) {
    frame->source = g_unix_fd_add_full(G_PRIORITY_DEFAULT, frame->acquire_fd,
                                       G_IO_IN | G_IO_ERR | G_IO_HUP, acquired,
                                       frame, NULL);
    frame->timeout = g_timeout_add_seconds(3, acquire_timeout, frame);
  } else
    frame_submit(frame);
  return TRUE;
}
static void cursor_name(WPEView *v, const char *name) {
  BrowserView *b = ((DemoView *)v)->owner;
  if (b) {
    g_free(b->cursor);
    b->cursor = g_strdup(name);
  }
}
static void demo_view_class_init(DemoViewClass *c) {
  c->render_buffer = render_buffer;
  c->set_cursor_from_name = cursor_name;
}
static void demo_view_init(DemoView *v) { v->owner = NULL; }

static gboolean load_failed(WebKitWebView *v, WebKitLoadEvent event,
                            const char *uri, GError *e, BrowserView *b) {
  (void)v;
  (void)event;
  (void)uri;
  b->pending_document_id = 0;
  if (!g_error_matches(e, WEBKIT_NETWORK_ERROR, WEBKIT_NETWORK_ERROR_CANCELLED))
    record_failure(b, 1, e->code, e->message, uri);
  return TRUE;
}
static void load_event_free(gpointer ptr) {
  BwLoadEvent *event = ptr;
  g_free(event->uri);
  g_free(event);
}
int bw_take_load_event(BrowserView *b, BwLoadEvent *result) {
  BwLoadEvent *event = g_queue_pop_head(&b->load_events);
  if (!event)
    return 0;
  *result = *event;
  g_free(event);
  return 1;
}
static gboolean network_uri(const char *uri);
static void load_changed(WebKitWebView *v, WebKitLoadEvent event,
                         BrowserView *b) {
  trace_native("load view=%p phase=%d", b, event);
  if (g_queue_get_length(&b->load_events) >= 64)
    load_event_free(g_queue_pop_head(&b->load_events));
  BwLoadEvent *entry = g_new0(BwLoadEvent, 1);
  entry->phase = event;
  entry->uri = g_strdup(webkit_web_view_get_uri(v));
  g_queue_push_tail(&b->load_events, entry);
  if (event == WEBKIT_LOAD_COMMITTED) {
    b->pending_document_id = 0;
    b->awaiting_commit = FALSE;
    if (network_uri(entry->uri)) {
      b->document_id = 0;
      g_clear_pointer(&b->document_uri, g_free);
      g_clear_pointer(&b->document_html, g_free);
    } else if (entry->uri && g_str_has_prefix(entry->uri, "gpui-document://")) {
      GUri *parsed = g_uri_parse(entry->uri, G_URI_FLAGS_NONE, NULL);
      const char *host = parsed ? g_uri_get_host(parsed) : NULL;
      if (host && g_hash_table_contains(b->documents, host) &&
          !g_strcmp0(g_uri_get_path(parsed), "/body")) {
        b->document_id = g_ascii_strtoull(host, NULL, 10);
        g_free(b->document_uri);
        b->document_uri =
            g_strdup_printf("gpui-document://%" PRIu64 "/body", b->document_id);
      }
      if (parsed)
        g_uri_unref(parsed);
    }
  }
  bw_wake();
}
static void terminated(WebKitWebView *v,
                       WebKitWebProcessTerminationReason reason,
                       BrowserView *b) {
  b->pending_document_id = 0;
  selection_cancel(b, "Web process terminated");
  cancel_owned_copies(b);
  record_failure(b, 2, reason, "Web process terminated. Reload to recover.",
                 webkit_web_view_get_uri(v));
}
static gboolean network_uri(const char *uri) {
  return uri && (g_str_has_prefix(uri, "https://") ||
                 g_str_has_prefix(uri, "http://"));
}
static void navigation_event_full(BrowserView *b, const char *uri,
                                  gboolean gesture, gboolean new_window,
                                  WebKitPolicyDecision *decision) {
  if (g_queue_get_length(&b->navigation_events) >= 64) {
    NavigationEntry *old = g_queue_pop_head(&b->navigation_events);
    if (old->token)
      g_hash_table_remove(b->navigation_pending, &old->token);
    navigation_free(old);
  }
  NavigationEntry *entry = g_new0(NavigationEntry, 1);
  entry->uri = g_strdup(uri);
  entry->flags = (gesture ? 1 : 0) | (new_window ? 2 : 0);
  if (decision && g_hash_table_size(b->navigation_pending) < 64) {
    entry->token = ++b->navigation_token;
    uint64_t *key = g_new(uint64_t, 1);
    *key = entry->token;
    g_hash_table_insert(b->navigation_pending, key, g_object_ref(decision));
  } else if (decision) {
    webkit_policy_decision_ignore(decision);
  }
  g_queue_push_tail(&b->navigation_events, entry);
  bw_wake();
}
static void navigation_event(BrowserView *b, const char *uri, gboolean gesture,
                             gboolean new_window) {
  navigation_event_full(b, uri, gesture, new_window, NULL);
}
static gboolean decide_policy(WebKitWebView *v, WebKitPolicyDecision *decision,
                              WebKitPolicyDecisionType type, BrowserView *b) {
  (void)v;
  if (type == WEBKIT_POLICY_DECISION_TYPE_RESPONSE) {
    if (!webkit_response_policy_decision_is_mime_type_supported(
            WEBKIT_RESPONSE_POLICY_DECISION(decision))) {
      webkit_policy_decision_ignore(decision);
      return TRUE;
    }
    return FALSE;
  }
  WebKitNavigationAction *action =
      webkit_navigation_policy_decision_get_navigation_action(
          WEBKIT_NAVIGATION_POLICY_DECISION(decision));
  const char *uri =
      webkit_uri_request_get_uri(webkit_navigation_action_get_request(action));
  gboolean gesture = webkit_navigation_action_is_user_gesture(action);
  gboolean same_document = b->document_uri &&
                           g_str_has_prefix(uri, b->document_uri) &&
                           (uri[strlen(b->document_uri)] == '\0' ||
                            uri[strlen(b->document_uri)] == '#');
  if (type != WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION && !b->restricted &&
      g_str_has_prefix(uri, "gpui-document://")) {
    GUri *parsed = g_uri_parse(uri, G_URI_FLAGS_NONE, NULL);
    const char *host = parsed ? g_uri_get_host(parsed) : NULL;
    if (host && g_hash_table_contains(b->documents, host) &&
        !g_strcmp0(g_uri_get_path(parsed), "/body")) {
      // Identity changes at the main-frame COMMITTED signal, never from a
      // subframe navigation policy request.
      g_uri_unref(parsed);
      return FALSE;
    }
    if (parsed)
      g_uri_unref(parsed);
  }
  if (type != WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION && same_document)
    return FALSE;
  if (type != WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION && !b->restricted &&
      network_uri(uri)) {
    gboolean approved = !g_strcmp0(b->approved_uri, uri);
    g_clear_pointer(&b->approved_uri, g_free);
    if (!b->mediate_navigation || approved)
      return FALSE;
    navigation_event_full(b, uri, gesture, FALSE, decision);
    return TRUE;
  }
  if (uri && !g_str_has_prefix(uri, "javascript:"))
    navigation_event(b, uri, gesture,
                     type == WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION);
  webkit_policy_decision_ignore(decision);
  return TRUE;
}
static void deny_download(WebKitNetworkSession *s, WebKitDownload *d,
                          gpointer data) {
  (void)s;
  (void)data;
  webkit_download_cancel(d);
}
static gboolean deny_permission(WebKitWebView *v, WebKitPermissionRequest *r,
                                gpointer data) {
  (void)v;
  (void)data;
  webkit_permission_request_deny(r);
  return TRUE;
}
static gboolean deny_dialog(WebKitWebView *v, WebKitScriptDialog *d,
                            gpointer data) {
  (void)v;
  (void)data;
  WebKitScriptDialogType type = webkit_script_dialog_get_dialog_type(d);
  if (type == WEBKIT_SCRIPT_DIALOG_CONFIRM ||
      type == WEBKIT_SCRIPT_DIALOG_BEFORE_UNLOAD_CONFIRM)
    webkit_script_dialog_confirm_set_confirmed(d, FALSE);
  else if (type == WEBKIT_SCRIPT_DIALOG_PROMPT)
    webkit_script_dialog_prompt_set_text(d, NULL);
  // Synchronous handling: WebKit unrefs its current-dialog pointer after the
  // signal returns, and the final unref closes the dialog. Closing inline
  // would clear that private pointer before WebKit unrefs it (2.54.2).
  return TRUE;
}
static gboolean deny_file_chooser(WebKitWebView *view,
                                  WebKitFileChooserRequest *request,
                                  gpointer data) {
  (void)view;
  (void)data;
  webkit_file_chooser_request_cancel(request);
  return TRUE;
}
static gboolean deny_authentication(WebKitWebView *view,
                                    WebKitAuthenticationRequest *request,
                                    gpointer data) {
  (void)view;
  (void)data;
  webkit_authentication_request_cancel(request);
  return TRUE;
}
static void resource_free(gpointer ptr) {
  Resource *r = ptr;
  g_bytes_unref(r->bytes);
  g_free(r->mime);
  g_free(r);
}
static void resource_error(WebKitURISchemeRequest *r, const char *message) {
  GError *e = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_NOT_FOUND, message);
  webkit_uri_scheme_request_finish_error(r, e);
  g_error_free(e);
}
static void resource_finish(WebKitURISchemeRequest *r, Resource *resource) {
  GInputStream *stream = g_memory_input_stream_new_from_bytes(resource->bytes);
  webkit_uri_scheme_request_finish(r, stream, g_bytes_get_size(resource->bytes),
                                   resource->mime);
  g_object_unref(stream);
}
static void resource_wait_free(ResourceWait *wait) {
  if (wait->timeout)
    g_source_remove(wait->timeout);
  g_object_unref(wait->request);
  g_free(wait->key);
  g_free(wait);
}
static gboolean resource_timeout(gpointer data) {
  ResourceWait *wait = data;
  wait->timeout = 0;
  resource_error(wait->request, "Application resource request timed out");
  g_hash_table_remove(wait->owner->resource_waits, wait->key);
  return G_SOURCE_REMOVE;
}
static gboolean valid_key(const char *key) {
  if (!key || !*key || strlen(key) > 160 || !strcmp(key, ".") ||
      !strcmp(key, ".."))
    return FALSE;
  for (const char *p = key; *p; p++)
    if (!g_ascii_isalnum(*p) && *p != '_' && *p != '-' && *p != '.')
      return FALSE;
  return TRUE;
}
static void resource_request(WebKitURISchemeRequest *r, gpointer data) {
  (void)data;
  BrowserView *b = g_object_get_data(
      G_OBJECT(webkit_uri_scheme_request_get_web_view(r)), "gpui-owner");
  if (!b) {
    resource_error(r, "View is closed");
    return;
  }
  GUri *uri =
      g_uri_parse(webkit_uri_scheme_request_get_uri(r), G_URI_FLAGS_NONE, NULL);
  if (!uri) {
    resource_error(r, "Invalid resource URI");
    return;
  }
  char *end = NULL;
  const char *host = g_uri_get_host(uri);
  uint64_t id = host ? g_ascii_strtoull(host, &end, 10) : 0;
  const char *path = g_uri_get_path(uri);
  const char *key = path && *path == '/' ? path + 1 : NULL;
  gboolean pending_body = id && id == b->pending_document_id && key &&
                          !strcmp(key, "body") &&
                          !g_strcmp0(g_uri_get_scheme(uri), "gpui-document") &&
                          g_hash_table_contains(b->documents, host);
  if ((!b->document_id && !pending_body) || !id || !end || *end ||
      (id != b->document_id && !pending_body &&
       (b->restricted || !g_hash_table_contains(b->documents, host))) ||
      g_uri_get_query(uri) || g_uri_get_userinfo(uri) ||
      g_uri_get_port(uri) != -1) {
    g_uri_unref(uri);
    resource_error(r, "Resource is outside the active document");
    return;
  }
  if (!strcmp(g_uri_get_scheme(uri), "gpui-document") && key &&
      !strcmp(key, "body") && g_hash_table_contains(b->documents, host)) {
    const char *html = g_hash_table_lookup(b->documents, host);
    GInputStream *stream = g_memory_input_stream_new_from_data(
        g_strdup(html), strlen(html), g_free);
    webkit_uri_scheme_request_finish(r, stream, strlen(html),
                                     "text/html; charset=utf-8");
    g_object_unref(stream);
    g_uri_unref(uri);
    return;
  }
  if (strcmp(g_uri_get_scheme(uri), "gpui-resource") || !valid_key(key)) {
    g_uri_unref(uri);
    resource_error(r, "Unknown application resource");
    return;
  }
  char *map_key = g_strdup_printf("%" PRIu64 "/%s", id, key);
  Resource *resource = g_hash_table_lookup(b->resources, map_key);
  if (resource) {
    resource_finish(r, resource);
    g_free(map_key);
    g_uri_unref(uri);
    return;
  }
  if (g_hash_table_size(b->resource_waits) >= 128 ||
      g_hash_table_contains(b->resource_waits, map_key)) {
    g_free(map_key);
    g_uri_unref(uri);
    resource_error(r, "Resource request limit reached");
    return;
  }
  ResourceWait *wait = g_new0(ResourceWait, 1);
  wait->owner = b;
  wait->request = g_object_ref(r);
  wait->key = g_strdup(map_key);
  wait->timeout = g_timeout_add_seconds(5, resource_timeout, wait);
  g_hash_table_insert(b->resource_waits, map_key, wait);
  g_queue_push_tail(&b->resource_events,
                    g_strdup_printf("%" PRIu64 "/%s", id, key));
  g_uri_unref(uri);
  bw_wake();
}
unsigned bw_editing_state(BrowserView *b) {
  WebKitEditorState *state = webkit_web_view_get_editor_state(b->web);
  return (webkit_editor_state_is_copy_available(state) ? 1 : 0) |
         (webkit_editor_state_is_cut_available(state) ? 2 : 0) |
         (webkit_editor_state_is_paste_available(state) ? 4 : 0) |
         (webkit_editor_state_is_undo_available(state) ? 8 : 0) |
         (webkit_editor_state_is_redo_available(state) ? 16 : 0);
}
static void mouse_target(WebKitWebView *web, WebKitHitTestResult *hit,
                         guint modifiers, BrowserView *b) {
  (void)web;
  (void)modifiers;
  g_set_object(&b->hit, hit);
  bw_wake();
}
static gboolean context_menu(WebKitWebView *web, WebKitContextMenu *menu,
                             WebKitHitTestResult *hit, BrowserView *b) {
  (void)web;
  g_set_object(&b->context_hit, hit);
  b->context_requested = TRUE;
  b->context_x = b->pointer_x;
  b->context_y = b->pointer_y;
  webkit_context_menu_get_position(menu, &b->context_x, &b->context_y);
  bw_wake();
  // The host draws its menu with GPUI. Never create a native popup above it.
  return TRUE;
}
static int hit_snapshot(BrowserView *b, WebKitHitTestResult *hit, int x, int y,
                        BwHitTest *result) {
  if (!hit || !result)
    return 0;
  *result = (BwHitTest){0};
  result->flags = (webkit_hit_test_result_context_is_link(hit) ? 1 : 0) |
                  (webkit_hit_test_result_context_is_image(hit) ? 2 : 0) |
                  (webkit_hit_test_result_context_is_media(hit) ? 4 : 0) |
                  (webkit_hit_test_result_context_is_editable(hit) ? 8 : 0) |
                  (webkit_hit_test_result_context_is_selection(hit) ? 16 : 0) |
                  (webkit_hit_test_result_context_is_scrollbar(hit) ? 32 : 0);
  result->editing_flags = bw_editing_state(b);
  result->x = x;
  result->y = y;
  result->link_uri = webkit_hit_test_result_get_link_uri(hit);
  result->image_uri = webkit_hit_test_result_get_image_uri(hit);
  result->media_uri = webkit_hit_test_result_get_media_uri(hit);
  result->link_title = webkit_hit_test_result_get_link_title(hit);
  return 1;
}
int bw_hit_test(BrowserView *b, BwHitTest *result) {
  return hit_snapshot(b, b->hit, b->pointer_x, b->pointer_y, result);
}
int bw_take_context_menu(BrowserView *b, BwHitTest *result) {
  if (!b->context_requested)
    return 0;
  b->context_requested = FALSE;
  return hit_snapshot(b, b->context_hit, b->context_x, b->context_y, result);
}
int bw_take_failure(BrowserView *b, BwFailure *result) {
  BwFailure *failure = g_queue_pop_head(&b->failures);
  if (!failure)
    return 0;
  *result = *failure;
  g_free(failure);
  return 1;
}
void bw_terminate_process(BrowserView *b) {
  webkit_web_view_terminate_web_process(b->web);
  bw_wake();
}
static void find_found(WebKitFindController *c, guint count, BrowserView *b) {
  (void)c;
  b->find_matches = count;
  b->find_pending = FALSE;
  bw_wake();
}
static void find_failed(WebKitFindController *c, BrowserView *b) {
  (void)c;
  b->find_matches = 0;
  b->find_pending = FALSE;
  bw_wake();
}

#ifdef BW_NATIVE_FAILURE_INJECTION
static unsigned diagnostic_constructions;
static gboolean diagnostic_failure(unsigned construction, const char *stage) {
  const char *requested = g_getenv("GPUI_WPE_FAIL_STAGE");
  const char *skip = g_getenv("GPUI_WPE_FAIL_SKIP");
  return requested &&
         construction > (skip ? g_ascii_strtoull(skip, NULL, 10) : 0) &&
         !strcmp(requested, stage);
}
#define BW_FAIL_STAGE(stage)                                                   \
  do {                                                                         \
    if (diagnostic_failure(construction, stage)) {                             \
      failure_stage = stage;                                                   \
      goto failed;                                                             \
    }                                                                          \
  } while (0)
#else
#define BW_FAIL_STAGE(stage) ((void)0)
#endif
static void cancel_navigation(BrowserView *b) {
  g_hash_table_remove_all(b->navigation_pending);
  g_queue_clear_full(&b->navigation_events, navigation_free);
}
static void cancel_resources(BrowserView *b);
BrowserView *bw_create(void *native_display, void *native_surface,
                       char **error) {
  const BwConfig config = {1, 0, 0, 0, {1., 1., 1., 1.}};
  return bw_create_config(native_display, native_surface, &config, error);
}
BrowserView *bw_create_config(void *native_display, void *native_surface,
                              const BwConfig *config, char **error) {
  if (config->javascript && shared_displays[0] &&
      shared_displays[0]->selection_owner) {
    if (error)
      *error = g_strdup(
          "Cannot create scripting-enabled view during selection query");
    return NULL;
  }
  if (config->theme != 0) {
    if (error)
      *error =
          g_strdup("Per-view color-scheme overrides are unsupported by WPE");
    return NULL;
  }
#ifdef BW_NATIVE_FAILURE_INJECTION
  unsigned construction = ++diagnostic_constructions;
#endif
  const char *failure_stage = NULL;
  BrowserView *b = g_new0(BrowserView, 1);
  b->surface = EGL_NO_SURFACE;
  b->gl = EGL_NO_CONTEXT;
  b->width = b->height = 1;
  b->clip_width = b->clip_height = 1;
  b->scale = 1;
  b->javascript = config->javascript;
  b->restricted = config->restricted_resources;
  b->mediate_navigation = config->mediate_navigation;
  unsigned pool_index = 0;
  b->navigation_pending = g_hash_table_new_full(g_int64_hash, g_int64_equal,
                                                g_free, navigation_deny);
  b->background = (WebKitColor){config->background[0], config->background[1],
                                config->background[2], config->background[3]};
  b->resources =
      g_hash_table_new_full(g_str_hash, g_str_equal, g_free, resource_free);
  b->documents = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_free);
  b->resource_waits = g_hash_table_new_full(g_str_hash, g_str_equal, g_free,
                                            (GDestroyNotify)resource_wait_free);
  b->imports = g_ptr_array_new();
  b->display = shared_displays[pool_index]
                   ? g_object_ref(shared_displays[pool_index])
                   : g_object_new(demo_display_get_type(), NULL);
  b->display->owner = b;
  BW_FAIL_STAGE("after-display");
  b->egl = eglGetDisplay(native_display);
  if (b->egl == EGL_NO_DISPLAY || !eglInitialize(b->egl, NULL, NULL))
    goto failed;
  eglBindAPI(EGL_OPENGL_ES_API);
  const EGLint attrs[] = {EGL_SURFACE_TYPE,
                          EGL_WINDOW_BIT,
                          EGL_RENDERABLE_TYPE,
                          EGL_OPENGL_ES2_BIT,
                          EGL_RED_SIZE,
                          8,
                          EGL_GREEN_SIZE,
                          8,
                          EGL_BLUE_SIZE,
                          8,
                          EGL_ALPHA_SIZE,
                          8,
                          EGL_NONE};
  EGLConfig egl_config;
  EGLint n;
  if (!eglChooseConfig(b->egl, attrs, &egl_config, 1, &n) || !n)
    goto failed;
  const EGLint ctx[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
  b->gl = eglCreateContext(b->egl, egl_config, EGL_NO_CONTEXT, ctx);
  b->window = wl_egl_window_create(native_surface, 1, 1);
  if (!b->window)
    goto failed;
  b->surface = eglCreateWindowSurface(b->egl, egl_config,
                                      (EGLNativeWindowType)b->window, NULL);
  if (b->gl == EGL_NO_CONTEXT || b->surface == EGL_NO_SURFACE ||
      !eglMakeCurrent(b->egl, b->surface, b->surface, b->gl))
    goto failed;
  eglSwapInterval(b->egl, 0);
  GLuint vs =
      shader(GL_VERTEX_SHADER,
             "attribute vec2 position; attribute vec2 uv; varying vec2 tex; "
             "void main(){gl_Position=vec4(position,0.,1.);tex=uv;}");
  GLuint fs =
      shader(GL_FRAGMENT_SHADER,
             "precision mediump float; varying vec2 tex; uniform sampler2D "
             "image; void main(){gl_FragColor=texture2D(image,tex);}");
  if (!vs || !fs)
    goto failed;
  b->program = glCreateProgram();
  glAttachShader(b->program, vs);
  glAttachShader(b->program, fs);
  glBindAttribLocation(b->program, 0, "position");
  glBindAttribLocation(b->program, 1, "uv");
  glLinkProgram(b->program);
  glDeleteShader(vs);
  glDeleteShader(fs);
  GLint linked;
  glGetProgramiv(b->program, GL_LINK_STATUS, &linked);
  if (!linked)
    goto failed;
  if (!eglGetProcAddress("eglCreateSyncKHR") ||
      !eglGetProcAddress("eglClientWaitSyncKHR") ||
      !eglGetProcAddress("eglDestroySyncKHR"))
    goto failed;
#ifdef BW_NATIVE_FAILURE_INJECTION
  if (diagnostic_failure(construction, "after-egl")) {
    snapshot_prepare(b, 1, 1);
    failure_stage = "after-egl";
    goto failed;
  }
#endif
  if (!b->display->formats) {
    PFNEGLGETPLATFORMDISPLAYEXTPROC platform_display =
        (void *)eglGetProcAddress("eglGetPlatformDisplayEXT");
    PFNEGLQUERYDISPLAYATTRIBEXTPROC query_engine_device =
        (void *)eglGetProcAddress("eglQueryDisplayAttribEXT");
    EGLAttrib engine_device = 0;
    if (platform_display && query_engine_device &&
        query_engine_device(b->egl, EGL_DEVICE_EXT, &engine_device))
      b->display->egl = platform_display(EGL_PLATFORM_DEVICE_EXT,
                                         (void *)engine_device, NULL);
    else if (platform_display)
      b->display->egl = platform_display(EGL_PLATFORM_SURFACELESS_MESA,
                                         EGL_DEFAULT_DISPLAY, NULL);
    if (b->display->egl == EGL_NO_DISPLAY ||
        !eglInitialize(b->display->egl, NULL, NULL))
      goto failed;

    PFNEGLQUERYDISPLAYATTRIBEXTPROC query_display =
        (void *)eglGetProcAddress("eglQueryDisplayAttribEXT");
    PFNEGLQUERYDEVICESTRINGEXTPROC query_device =
        (void *)eglGetProcAddress("eglQueryDeviceStringEXT");
    EGLAttrib device;
    if (query_display && query_device &&
        query_display(b->egl, EGL_DEVICE_EXT, &device)) {
      const char *node =
          query_device((EGLDeviceEXT)device, EGL_DRM_RENDER_NODE_FILE_EXT);
      const char *primary =
          query_device((EGLDeviceEXT)device, EGL_DRM_DEVICE_FILE_EXT);
      if (node && primary)
        b->display->device = wpe_drm_device_new(primary, node);
    }
    /* Query actual importable formats/modifiers. Exclude external-only formats
     * because this presenter samples a normal GL_TEXTURE_2D. */
    PFNEGLQUERYDMABUFFORMATSEXTPROC query_formats =
        (void *)eglGetProcAddress("eglQueryDmaBufFormatsEXT");
    PFNEGLQUERYDMABUFMODIFIERSEXTPROC query_mods =
        (void *)eglGetProcAddress("eglQueryDmaBufModifiersEXT");
    WPEBufferFormatsBuilder *builder =
        wpe_buffer_formats_builder_new(b->display->device);
    wpe_buffer_formats_builder_append_group(builder, b->display->device,
                                            WPE_BUFFER_FORMAT_USAGE_RENDERING);
    EGLint num = 0;
    if (query_formats && query_mods && query_formats(b->egl, 0, NULL, &num)) {
      EGLint *formats = g_new(EGLint, num);
      query_formats(b->egl, num, formats, &num);
      for (int i = 0; i < num; i++) {
        if (formats[i] != DRM_FORMAT_ARGB8888 &&
            formats[i] != DRM_FORMAT_XRGB8888)
          continue;
        EGLint nm = 0;
        query_mods(b->egl, formats[i], 0, NULL, NULL, &nm);
        EGLuint64KHR *mods = g_new(EGLuint64KHR, nm);
        EGLBoolean *external = g_new(EGLBoolean, nm);
        if (query_mods(b->egl, formats[i], nm, mods, external, &nm))
          for (int m = 0; m < nm; m++)
            if (!external[m])
              wpe_buffer_formats_builder_append_format(builder, formats[i],
                                                       mods[m]);
        g_free(mods);
        g_free(external);
      }
      g_free(formats);
    }
    b->display->formats = wpe_buffer_formats_builder_end(builder);
    wpe_buffer_formats_builder_unref(builder);
    if (!wpe_display_connect(WPE_DISPLAY(b->display), NULL))
      goto failed;
    if (!shared_displays[pool_index])
      shared_displays[pool_index] = g_object_ref(b->display);
  }
  wpe_display_set_available_input_devices(
      WPE_DISPLAY(b->display),
      WPE_AVAILABLE_INPUT_DEVICE_MOUSE | WPE_AVAILABLE_INPUT_DEVICE_KEYBOARD);
  b->context = webkit_web_context_new();
  webkit_web_context_register_uri_scheme(b->context, "gpui-document",
                                         resource_request, NULL, NULL);
  webkit_web_context_register_uri_scheme(b->context, "gpui-resource",
                                         resource_request, NULL, NULL);
  WebKitSecurityManager *security =
      webkit_web_context_get_security_manager(b->context);
  webkit_security_manager_register_uri_scheme_as_local(security,
                                                       "gpui-document");
  webkit_security_manager_register_uri_scheme_as_local(security,
                                                       "gpui-resource");
  webkit_security_manager_register_uri_scheme_as_cors_enabled(security,
                                                              "gpui-resource");
  b->session = webkit_network_session_new_ephemeral();
  g_signal_connect(b->session, "download-started", G_CALLBACK(deny_download),
                   NULL);
  BW_FAIL_STAGE("before-webview");
  WebKitSettings *settings = webkit_settings_new();
  webkit_settings_set_enable_javascript(settings, b->javascript);
  /* The deprecated DNS-prefetch setting is a no-op in WPE 2.54. Transport
   * hints are enforced by the content filter, including ResourceType::Ping. */
  b->content_manager = webkit_user_content_manager_new();
  if (b->restricted) {
    WebKitFeatureList *features = webkit_settings_get_all_features();
    WebKitFeature *preconnect = webkit_feature_list_find(features, "LinkPreconnect");
    if (preconnect)
      webkit_settings_set_feature_enabled(settings, preconnect, FALSE);
    gboolean disabled = preconnect &&
        !webkit_settings_get_feature_enabled(settings, preconnect);
    webkit_feature_list_unref(features);
    if (!disabled) {
      g_object_unref(settings);
      b->error = g_strdup("Cannot disable WebKit speculative preconnect");
      goto failed;
    }
    ResourcePolicyJob *job = policy_ensure();
    b->policy_generation = job->generation;
    if (job->error) {
      g_object_unref(settings);
      b->error = g_strdup(job->error);
      goto failed;
    }
    if (job->filter) {
      webkit_user_content_manager_add_filter(b->content_manager, job->filter);
      b->policy_ready = TRUE;
    }
    webkit_settings_set_enable_media_stream(settings, FALSE);
    webkit_settings_set_enable_webrtc(settings, FALSE);
    webkit_settings_set_enable_webaudio(settings, FALSE);
    webkit_settings_set_enable_webgl(settings, FALSE);
  }
  webkit_settings_set_allow_file_access_from_file_urls(settings, FALSE);
  webkit_settings_set_allow_universal_access_from_file_urls(settings, FALSE);
  webkit_settings_set_javascript_can_access_clipboard(settings, FALSE);
  webkit_settings_set_media_playback_requires_user_gesture(settings, TRUE);
  // The host has no fullscreen integration yet; reject requests before any
  // engine/native fullscreen transition or unhandled UI can begin.
  webkit_settings_set_enable_fullscreen(settings, FALSE);
  const char *csp =
      b->restricted
          ? "default-src 'none'; script-src 'none'; connect-src 'none'; "
            "img-src gpui-resource: data:; style-src 'unsafe-inline' "
            "gpui-resource:; font-src gpui-resource:; frame-src 'none'; "
            "object-src 'none'; base-uri 'none'; form-action 'none'"
          : NULL;
  b->web =
      g_object_new(WEBKIT_TYPE_WEB_VIEW, "display", b->display, "web-context",
                   b->context, "network-session", b->session, "settings",
                   settings, "user-content-manager", b->content_manager,
                   "default-content-security-policy", csp, NULL);
  g_object_unref(settings);
  webkit_web_view_set_background_color(b->web, &b->background);
  b->display->owner = NULL;
  g_object_set_data(G_OBJECT(b->web), "gpui-owner", b);
  b->view = webkit_web_view_get_wpe_view(b->web);
  ((DemoView *)b->view)->owner = b;
  b->toplevel = wpe_display_create_toplevel(WPE_DISPLAY(b->display), 1);
  wpe_view_set_toplevel(b->view, b->toplevel);
  g_signal_connect(b->web, "mouse-target-changed", G_CALLBACK(mouse_target), b);
  g_signal_connect(b->web, "context-menu", G_CALLBACK(context_menu), b);
  g_signal_connect(b->web, "load-changed", G_CALLBACK(load_changed), b);
  g_signal_connect(b->web, "load-failed", G_CALLBACK(load_failed), b);
  g_signal_connect(b->web, "web-process-terminated", G_CALLBACK(terminated), b);
  g_signal_connect(b->web, "decide-policy", G_CALLBACK(decide_policy), b);
  g_signal_connect(b->web, "permission-request", G_CALLBACK(deny_permission),
                   NULL);
  g_signal_connect(b->web, "script-dialog", G_CALLBACK(deny_dialog), NULL);
  g_signal_connect(b->web, "run-file-chooser", G_CALLBACK(deny_file_chooser),
                   NULL);
  g_signal_connect(b->web, "authenticate", G_CALLBACK(deny_authentication),
                   NULL);
  WebKitFindController *find = webkit_web_view_get_find_controller(b->web);
  g_signal_connect(find, "found-text", G_CALLBACK(find_found), b);
  g_signal_connect(find, "counted-matches", G_CALLBACK(find_found), b);
  g_signal_connect(find, "failed-to-find-text", G_CALLBACK(find_failed), b);
  b->clipboard_count = wpe_clipboard_get_change_count(
      display_clipboard(WPE_DISPLAY(b->display)));
  b->registered = 1;
  live_views++;
  clipboard_scripts_used |= !!b->javascript;
  if (!live_registry)
    live_registry = g_ptr_array_new();
  g_ptr_array_add(live_registry, b);
  /* Also adopt completion if a native constructor ran a nested main loop
   * before this view joined the callback registry. */
  if (b->restricted)
    policy_notify(resource_policy);
  BW_FAIL_STAGE("after-webview");
  return b;
failed:
  if (error)
    *error = failure_stage
                 ? g_strdup_printf("Injected native constructor failure at %s",
                                   failure_stage)
                 : b->error ? g_strdup(b->error) : g_strdup_printf(
                       "Cannot initialize WPE/EGL presenter (EGL error 0x%x)",
                       eglGetError());
  bw_destroy(b);
  return NULL;
}
void bw_destroy(BrowserView *b) {
  bw_wake();
  if (!b)
    return;
  b->policy_generation = 0;
  b->policy_load_queued = FALSE;
  selection_cancel(b, "View closed");
  if (b->display)
    cancel_owned_copies(b);
  if (b->display) {
    GList *link = b->display->deferred.head;
    while (link) {
      GList *next = link->next;
      DeferredInput *input = link->data;
      if (input->owner == b) {
        g_queue_delete_link(&b->display->deferred, link);
        deferred_free(input);
      }
      link = next;
    }
  }
  if (b->completion_source) {
    g_source_remove(b->completion_source);
    b->completion_source = 0;
  }
  if (b->gl != EGL_NO_CONTEXT && b->surface != EGL_NO_SURFACE) {
    gboolean current = eglMakeCurrent(b->egl, b->surface, b->surface, b->gl);
    PFNEGLCLIENTWAITSYNCKHRPROC wait =
        (void *)eglGetProcAddress("eglClientWaitSyncKHR");
    while (!g_queue_is_empty(&b->frames_pending)) {
      Frame *frame = g_queue_peek_head(&b->frames_pending);
      // Close is the only bounded drain; the normal renderer never blocks.
      if (frame->sync != EGL_NO_SYNC_KHR &&
          wait(b->egl, frame->sync, 0, 1000000000) !=
              EGL_CONDITION_SATISFIED_KHR)
        g_error("GPU did not complete during bounded teardown; refusing unsafe "
                "buffer release");
      frame_release(frame);
    }
    if (b->imports) {
      for (guint i = 0; i < b->imports->len; i++) {
        BufferImport *entry = g_ptr_array_index(b->imports, i);
        // A failed make-current must never delete overlapping texture names in
        // another view's context. Context destruction frees these GL objects.
        if (!current)
          entry->texture = 0;
        import_free(b, entry);
      }
      g_ptr_array_set_size(b->imports, 0);
    }
  }
  if (b->web) {
    cancel_navigation(b);
    cancel_resources(b);
    g_signal_handlers_disconnect_by_data(
        webkit_web_view_get_find_controller(b->web), b);
    g_object_set_data(G_OBJECT(b->web), "gpui-owner", NULL);
    g_signal_handlers_disconnect_by_data(b->web, b);
    if (b->im)
      g_object_remove_weak_pointer(G_OBJECT(b->im), (gpointer *)&b->im);
    b->im = NULL;
    ((DemoView *)b->view)->owner = NULL;
    wpe_view_unmap(b->view);
    webkit_web_view_stop_loading(b->web);
    g_clear_object(&b->web);
  }
  g_clear_object(&b->toplevel);
  g_clear_object(&b->session);
  g_clear_object(&b->context);
  g_clear_object(&b->content_manager);
  if (b->gl != EGL_NO_CONTEXT && b->surface != EGL_NO_SURFACE &&
      eglMakeCurrent(b->egl, b->surface, b->surface, b->gl)) {
    glDeleteTextures(1, &b->texture);
    if (b->snapshot_texture)
      glDeleteTextures(1, &b->snapshot_texture);
    if (b->snapshot_fbo)
      glDeleteFramebuffers(1, &b->snapshot_fbo);
    if (b->program)
      glDeleteProgram(b->program);
    eglMakeCurrent(b->egl, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
  }
  if (b->surface != EGL_NO_SURFACE)
    eglDestroySurface(b->egl, b->surface);
  if (b->gl != EGL_NO_CONTEXT)
    eglDestroyContext(b->egl, b->gl);
  if (b->window)
    wl_egl_window_destroy(b->window);
  if (b->display && b->display->owner == b)
    b->display->owner = NULL;
  if (b->registered) {
    live_views--;
    g_ptr_array_remove_fast(live_registry, b);
  }
  g_clear_object(&b->display);
  g_free(b->error);
  g_free(b->cursor);
  if (b->imports)
    g_ptr_array_unref(b->imports);
  g_free(b->document_html);
  g_free(b->document_uri);
  g_free(b->approved_uri);
  g_free(b->selection_result);
  g_clear_object(&b->hit);
  g_clear_object(&b->context_hit);
  g_queue_clear_full(&b->failures, failure_free);
  g_queue_clear_full(&b->load_events, load_event_free);
  if (b->navigation_pending)
    g_hash_table_unref(b->navigation_pending);
  if (b->resource_waits)
    g_hash_table_unref(b->resource_waits);
  if (b->resources)
    g_hash_table_unref(b->resources);
  if (b->documents)
    g_hash_table_unref(b->documents);
  g_queue_clear_full(&b->document_order, g_free);
  g_queue_clear_full(&b->resource_events, g_free);
  g_queue_clear_full(&b->navigation_events, navigation_free);
  g_free(b);
}
void bw_free(void *p) { g_free(p); }
void bw_pump(void) {
  int64_t started = g_get_monotonic_time();
  for (int i = 0; i < 32 && g_get_monotonic_time() - started < 2000 &&
                  g_main_context_pending(NULL);
       i++)
    g_main_context_iteration(NULL, FALSE);
}
void bw_geometry(BrowserView *b, int w, int h, double scale, int visible) {
  visible = visible && w > 0 && h > 0;
  if (w < 0 || h < 0 || !isfinite(scale) || scale <= 0 ||
      ceil(w * scale) > 16384 || ceil(h * scale) > 16384 ||
      ceil(w * scale) * ceil(h * scale) > 16777216) {
    set_error(b, "Viewport exceeds supported dimensions");
    return;
  }
  if (b->width != w || b->height != h || b->scale != scale) {
    trace_native("geometry view=%p size=%dx%d scale=%.3f", b, w, h, scale);
    b->metrics.geometry_generation++;
    b->metrics.geometry_us = g_get_monotonic_time();
    b->metrics.width = w;
    b->metrics.height = h;
    b->width = w;
    b->height = h;
    b->scale = scale;
    wl_egl_window_resize(
        b->window, MAX(1, (int)ceil((b->clip_set ? b->clip_width : w) * scale)),
        MAX(1, (int)ceil((b->clip_set ? b->clip_height : h) * scale)), 0, 0);
    wpe_toplevel_scale_changed(b->toplevel, scale);
    wpe_toplevel_resized(b->toplevel, MAX(w, 1), MAX(h, 1));
    wpe_view_resized(b->view, MAX(w, 1), MAX(h, 1));
    bw_wake();
  }
  if (b->visible != visible) {
    trace_native("visibility view=%p visible=%d", b, visible);
    b->visible = visible;
    bw_wake();
    wpe_view_set_visible(b->view, visible);
    if (visible)
      wpe_view_map(b->view);
    else {
      wpe_view_focus_out(b->view);
      wpe_view_unmap(b->view);
    }
  }
}
void bw_focus(BrowserView *b, int focused) {
  if (wpe_view_get_has_focus(b->view) == !!focused)
    return;
  trace_native("focus view=%p focused=%d", b, focused);
  wpe_toplevel_state_changed(b->toplevel,
                             focused ? WPE_TOPLEVEL_STATE_ACTIVE : 0);
  bw_wake();
  if (focused)
    wpe_view_focus_in(b->view);
  else
    wpe_view_focus_out(b->view);
}
static void cancel_resources(BrowserView *b) {
  GHashTableIter it;
  gpointer value;
  g_hash_table_iter_init(&it, b->resource_waits);
  while (g_hash_table_iter_next(&it, NULL, &value))
    resource_error(((ResourceWait *)value)->request,
                   "Document replaced or closed");
  g_hash_table_remove_all(b->resource_waits);
  g_queue_clear_full(&b->resource_events, g_free);
}
void bw_load(BrowserView *b, const char *uri) {
  if (b->restricted || !network_uri(uri)) {
    navigation_event(b, uri ? uri : "", FALSE, FALSE);
    return;
  }
  selection_cancel(b, "Document replaced");
  cancel_owned_copies(b);
  cancel_navigation(b);
  cancel_resources(b);
  b->document_id = 0;
  b->pending_document_id = 0;
  g_clear_pointer(&b->document_uri, g_free);
  g_clear_pointer(&b->document_html, g_free);
  // Retain bounded history storage, but identity0 makes the scheme handler
  // reject every application resource while ordinary network content is active.
  g_clear_pointer(&b->error, g_free);
  bw_find_clear(b);
  g_free(b->approved_uri);
  b->approved_uri = g_strdup(uri);
  webkit_web_view_load_uri(b->web, uri);
  bw_wake();
}
static void replace_document_surface(BrowserView *b) {
  b->snapshot_document_id = 0;
  b->snapshot_valid = FALSE;
  GList *link = b->frames_pending.head;
  while (link) {
    Frame *frame = link->data;
    link = link->next;
    if (!frame->rendered)
      frame_release(frame);
  }
  if (b->visible && eglMakeCurrent(b->egl, b->surface, b->surface, b->gl)) {
    glBindFramebuffer(GL_FRAMEBUFFER, 0);
    glClearColor(b->background.red, b->background.green, b->background.blue,
                 b->background.alpha);
    glClear(GL_COLOR_BUFFER_BIT);
    glFlush();
    eglSwapBuffers(b->egl, b->surface);
  }
}
static gboolean document_valid(BrowserView *b, uint64_t id, const char *html) {
  if (b->policy_failed)
    return FALSE;
  if (!id || !html || strlen(html) > 8 * 1024 * 1024)
    return FALSE;
  char *key = g_strdup_printf("%" PRIu64, id);
  const char *old = g_hash_table_lookup(b->documents, key);
  gboolean valid = !old || !strcmp(old, html);
  g_free(key);
  return valid;
}
static gboolean resource_valid(uint64_t id, const char *key,
                               const unsigned char *bytes, size_t length,
                               const char *mime) {
  return id && valid_key(key) && bytes && length <= 32 * 1024 * 1024 && mime &&
         *mime && strlen(mime) <= 128 && !strchr(mime, '\n') &&
         !strchr(mime, '\r');
}
int bw_load_document_resources(BrowserView *b, uint64_t id, const char *html,
                               const BwResource *entries, size_t count) {
  if (!document_valid(b, id, html) || count > 1024 || (count && !entries))
    return 0;
  GHashTable *candidate =
      g_hash_table_new_full(g_str_hash, g_str_equal, g_free, resource_free);
  char *prefix = g_strdup_printf("%" PRIu64 "/", id);
  size_t total = 0;
  GHashTableIter it;
  gpointer key, value;
  if (!b->restricted) {
    g_hash_table_iter_init(&it, b->resources);
    while (g_hash_table_iter_next(&it, &key, &value)) {
      if (g_str_has_prefix(key, prefix))
        continue;
      Resource *old = value, *copy = g_new0(Resource, 1);
      copy->bytes = g_bytes_ref(old->bytes);
      copy->mime = g_strdup(old->mime);
      total += g_bytes_get_size(copy->bytes);
      g_hash_table_insert(candidate, g_strdup(key), copy);
    }
  }
  for (size_t i = 0; i < count; i++) {
    const BwResource *entry = entries + i;
    if (!resource_valid(id, entry->key, entry->bytes, entry->length,
                        entry->mime))
      goto invalid;
    char *map_key = g_strconcat(prefix, entry->key, NULL);
    if (g_hash_table_contains(candidate, map_key)) {
      g_free(map_key);
      goto invalid;
    }
    total += entry->length;
    if (total > 64 * 1024 * 1024 || g_hash_table_size(candidate) >= 1024) {
      g_free(map_key);
      goto invalid;
    }
    Resource *resource = g_new0(Resource, 1);
    resource->bytes = g_bytes_new(entry->bytes, entry->length);
    resource->mime = g_strdup(entry->mime);
    g_hash_table_insert(candidate, map_key, resource);
  }
  if (total > 64 * 1024 * 1024 || g_hash_table_size(candidate) > 1024)
    goto invalid;
  // All rejecting validation precedes mutation. This UI-thread transaction
  // cannot interleave resource callbacks or leave the Rust/native IDs apart.
  GHashTable *old_resources = b->resources;
  b->resources = candidate;
  int result = bw_load_document(b, id, html);
  g_assert(result);
  g_hash_table_unref(old_resources);
  g_free(prefix);
  return 1;
invalid:
  g_hash_table_unref(candidate);
  g_free(prefix);
  return 0;
}
int bw_load_document(BrowserView *b, uint64_t id, const char *html) {
  if (b->policy_failed)
    return 0;
  if (!id || !html || strlen(html) > 8 * 1024 * 1024) {
    set_error(b, "Invalid or oversized document");
    return 0;
  }
  char *id_key = g_strdup_printf("%" PRIu64, id);
  const char *old = g_hash_table_lookup(b->documents, id_key);
  if (old && strcmp(old, html)) {
    g_free(id_key);
    set_error(b, "Document identity cannot be reused for different HTML");
    return 0;
  }
  selection_cancel(b, "Document replaced");
  cancel_owned_copies(b);
  cancel_navigation(b);
  cancel_resources(b);
  replace_document_surface(b);
  b->awaiting_commit = TRUE;
  g_clear_pointer(&b->approved_uri, g_free);
  if (b->restricted) {
    g_hash_table_remove_all(b->documents);
    g_queue_clear_full(&b->document_order, g_free);
  }
  if (!g_hash_table_contains(b->documents, id_key)) {
    g_hash_table_insert(b->documents, g_strdup(id_key), g_strdup(html));
  }
  for (GList *link = b->document_order.head; link; link = link->next) {
    if (!strcmp(link->data, id_key)) {
      g_free(link->data);
      g_queue_delete_link(&b->document_order, link);
      break;
    }
  }
  g_queue_push_tail(&b->document_order, g_strdup(id_key));
  while (g_queue_get_length(&b->document_order) > 8) {
    char *victim = g_queue_pop_head(&b->document_order);
    g_hash_table_remove(b->documents, victim);
    char *prefix = g_strconcat(victim, "/", NULL);
    GHashTableIter it;
    gpointer key;
    g_hash_table_iter_init(&it, b->resources);
    while (g_hash_table_iter_next(&it, &key, NULL))
      if (g_str_has_prefix(key, prefix))
        g_hash_table_iter_remove(&it);
    g_free(prefix);
    g_free(victim);
  }
  g_free(id_key);
  b->document_id = id;
  b->pending_document_id = 0;
  g_free(b->document_html);
  b->document_html = g_strdup(html);
  g_free(b->document_uri);
  b->document_uri = g_strdup_printf("gpui-document://%" PRIu64 "/body", id);
  if (b->restricted) {
    GHashTableIter it;
    gpointer key;
    g_hash_table_iter_init(&it, b->resources);
    char *prefix = g_strdup_printf("%" PRIu64 "/", id);
    while (g_hash_table_iter_next(&it, &key, NULL))
      if (!g_str_has_prefix(key, prefix))
        g_hash_table_iter_remove(&it);
    g_free(prefix);
  }
  g_clear_pointer(&b->error, g_free);
  bw_find_clear(b);
  if (b->restricted && !b->policy_ready)
    b->policy_load_queued = TRUE;
  else
    webkit_web_view_load_uri(b->web, b->document_uri);
  bw_wake();
  return 1;
}
int bw_set_resource(BrowserView *b, uint64_t id, const char *key,
                    const unsigned char *bytes, size_t length,
                    const char *mime) {
  if (!resource_valid(id, key, bytes, length, mime))
    return 0;
  char *map_key = g_strdup_printf("%" PRIu64 "/%s", id, key);
  Resource *existing = g_hash_table_lookup(b->resources, map_key);
  size_t total = length;
  GHashTableIter it;
  gpointer value;
  g_hash_table_iter_init(&it, b->resources);
  while (g_hash_table_iter_next(&it, NULL, &value))
    if (value != existing)
      total += g_bytes_get_size(((Resource *)value)->bytes);
  if (total > 64 * 1024 * 1024 ||
      (!existing && g_hash_table_size(b->resources) >= 1024)) {
    g_free(map_key);
    return 0;
  }
  Resource *resource = g_new0(Resource, 1);
  resource->bytes = g_bytes_new(bytes, length);
  resource->mime = g_strdup(mime);
  g_hash_table_replace(b->resources, g_strdup(map_key), resource);
  ResourceWait *wait = g_hash_table_lookup(b->resource_waits, map_key);
  if (wait) {
    resource_finish(wait->request, resource);
    g_hash_table_remove(b->resource_waits, map_key);
  }
  g_free(map_key);
  bw_wake();
  return 1;
}
void bw_fail_resource(BrowserView *b, uint64_t id, const char *key) {
  char *map_key = g_strdup_printf("%" PRIu64 "/%s", id, key);
  ResourceWait *wait = g_hash_table_lookup(b->resource_waits, map_key);
  if (wait) {
    resource_error(wait->request,
                   "Application resource was denied or not found");
    g_hash_table_remove(b->resource_waits, map_key);
  }
  g_free(map_key);
  bw_wake();
}
char *bw_take_resource_request(BrowserView *b) {
  return g_queue_pop_head(&b->resource_events);
}
static void authorize_history_body(BrowserView *b, int action) {
  b->pending_document_id = 0;
  WebKitBackForwardList *history =
      webkit_web_view_get_back_forward_list(b->web);
  WebKitBackForwardListItem *item =
      action == 0   ? webkit_back_forward_list_get_back_item(history)
      : action == 1 ? webkit_back_forward_list_get_forward_item(history)
                    : webkit_back_forward_list_get_current_item(history);
  const char *uri = item ? webkit_back_forward_list_item_get_uri(item) : NULL;
  if (!uri || !g_str_has_prefix(uri, "gpui-document://"))
    return;
  GUri *parsed = g_uri_parse(uri, G_URI_FLAGS_NONE, NULL);
  const char *host = parsed ? g_uri_get_host(parsed) : NULL;
  if (host && g_hash_table_contains(b->documents, host) &&
      !g_strcmp0(g_uri_get_path(parsed), "/body") && !g_uri_get_query(parsed) &&
      !g_uri_get_userinfo(parsed) && g_uri_get_port(parsed) == -1)
    b->pending_document_id = g_ascii_strtoull(host, NULL, 10);
  if (parsed)
    g_uri_unref(parsed);
}
void bw_action(BrowserView *b, int action) {
  trace_native("action view=%p action=%d", b, action);
  if (b->restricted && !b->policy_ready && action <= 3) {
    if (action == 2 && !b->policy_failed && b->document_uri)
      b->policy_load_queued = TRUE;
    if (action == 3)
      b->policy_load_queued = FALSE;
    bw_wake();
    return;
  }
  if (action >= 4 && action <= 6 && defer_input(b, 0, action, NULL, 0, 0))
    return;
  if ((action == 4 && (bw_editing_state(b) & 1)) ||
      (action == 5 && (bw_editing_state(b) & 2)))
    if (!copy_started(b))
      return;
  bw_wake();
  switch (action) {
  case 0:
    authorize_history_body(b, action);
    webkit_web_view_go_back(b->web);
    break;
  case 1:
    authorize_history_body(b, action);
    webkit_web_view_go_forward(b->web);
    break;
  case 2:
    authorize_history_body(b, action);
    selection_cancel(b, "Document reloaded");
    cancel_owned_copies(b);
    cancel_navigation(b);
    cancel_resources(b);
    bw_find_clear(b);
    g_clear_pointer(&b->error, g_free);
    webkit_web_view_reload(b->web);
    break;
  case 3:
    b->pending_document_id = 0;
    webkit_web_view_stop_loading(b->web);
    break;
  default: {
    const char *commands[] = {"Copy",      "Cut",  "Paste",
                              "SelectAll", "Undo", "Redo"};
    if (action >= 4 && action < 10)
      webkit_web_view_execute_editing_command(b->web, commands[action - 4]);
  }
  }
}
void bw_zoom(BrowserView *b, double zoom) {
  bw_wake();
  webkit_web_view_set_zoom_level(b->web, zoom);
}
const char *bw_uri(BrowserView *b) { return webkit_web_view_get_uri(b->web); }
const char *bw_title(BrowserView *b) {
  return webkit_web_view_get_title(b->web);
}
const char *bw_error(BrowserView *b) { return b->error; }
char *bw_take_navigation(BrowserView *b, unsigned *flags, uint64_t *token) {
  NavigationEntry *entry = g_queue_pop_head(&b->navigation_events);
  if (!entry)
    return NULL;
  if (flags)
    *flags = entry->flags;
  if (token)
    *token = entry->token;
  char *uri = g_strdup(entry->uri);
  navigation_free(entry);
  return uri;
}
void bw_resolve_navigation(BrowserView *b, uint64_t token, int allow) {
  gpointer key = NULL, value = NULL;
  if (!token || !g_hash_table_lookup_extended(b->navigation_pending, &token,
                                              &key, &value))
    return;
  g_hash_table_steal(b->navigation_pending, key);
  g_free(key);
  if (allow)
    webkit_policy_decision_use(value);
  else
    webkit_policy_decision_ignore(value);
  g_object_unref(value);
  bw_wake();
}
char *bw_take_new_uri(BrowserView *b) {
  uint64_t token = 0;
  char *uri = bw_take_navigation(b, NULL, &token);
  bw_resolve_navigation(b, token, FALSE);
  return uri;
}
int bw_state(BrowserView *b) {
  return (webkit_web_view_is_loading(b->web) || b->policy_load_queued) |
         (webkit_web_view_can_go_back(b->web) << 1) |
         (webkit_web_view_can_go_forward(b->web) << 2);
}
uint64_t bw_frames(BrowserView *b) { return b->frames; }
uint64_t bw_document_id(BrowserView *b) { return b->document_id; }
const char *bw_cursor(BrowserView *b) { return b->cursor; }
void bw_pointer(BrowserView *b, int kind, double x, double y, int button,
                uint32_t mods, int count) {
  trace_native("pointer view=%p kind=%d pos=%.1f,%.1f", b, kind, x, y);
  WPEEvent *e;
  b->pointer_x = (int)x;
  b->pointer_y = (int)y;
  if (kind < 2)
    e = wpe_event_pointer_button_new(
        kind == 0 ? WPE_EVENT_POINTER_DOWN : WPE_EVENT_POINTER_UP, b->view,
        WPE_INPUT_SOURCE_MOUSE, event_time(), mods, button, x, y,
        kind == 0 ? count : 0);
  else
    e = wpe_event_pointer_move_new(
        kind == 3 ? WPE_EVENT_POINTER_LEAVE : WPE_EVENT_POINTER_MOVE, b->view,
        WPE_INPUT_SOURCE_MOUSE, event_time(), mods, x, y, 0, 0);
  if (e) {
    bw_wake();
    wpe_view_event(b->view, e);
    wpe_event_unref(e);
  }
}
void bw_scroll(BrowserView *b, double x, double y, double dx, double dy,
               uint32_t mods) {
  trace_native("scroll view=%p delta=%.1f,%.1f", b, dx, dy);
  WPEEvent *e =
      wpe_event_scroll_new(b->view, WPE_INPUT_SOURCE_TOUCHPAD, event_time(),
                           mods, dx, dy, TRUE, dx == 0 && dy == 0, x, y);
  if (e) {
    bw_wake();
    wpe_view_event(b->view, e);
    wpe_event_unref(e);
  }
}
void bw_key(BrowserView *b, const char *key, uint32_t mods, int down) {
  gboolean clipboard_shortcut =
      (mods & WPE_MODIFIER_KEYBOARD_CONTROL) &&
      (!g_ascii_strcasecmp(key, "c") || !g_ascii_strcasecmp(key, "x") ||
       !g_ascii_strcasecmp(key, "v"));
  if ((!b->display->clipboard_poisoned || clipboard_shortcut) &&
      defer_input(b, 1, 0, key, mods, down))
    return;
  if (down && (mods & WPE_MODIFIER_KEYBOARD_CONTROL) &&
      ((!g_ascii_strcasecmp(key, "c") && (bw_editing_state(b) & 1)) ||
       (!g_ascii_strcasecmp(key, "x") && (bw_editing_state(b) & 2))))
    if (!copy_started(b))
      return;
  xkb_keysym_t sym = xkb_keysym_from_name(key, XKB_KEYSYM_CASE_INSENSITIVE);
  if (sym == XKB_KEY_NoSymbol && g_utf8_strlen(key, -1) == 1)
    sym = xkb_utf32_to_keysym(g_utf8_get_char(key));
  if (sym == XKB_KEY_NoSymbol)
    return;
  WPEEvent *e = wpe_event_keyboard_new(
      down ? WPE_EVENT_KEYBOARD_KEY_DOWN : WPE_EVENT_KEYBOARD_KEY_UP, b->view,
      WPE_INPUT_SOURCE_KEYBOARD, event_time(), mods, 0, sym);
  if (e) {
    bw_wake();
    wpe_view_event(b->view, e);
    wpe_event_unref(e);
  }
}
void bw_text(BrowserView *b, const char *text, int preedit) {
  trace_native("text view=%p bytes=%zu preedit=%d", b, strlen(text), preedit);
  bw_wake();
  if (!b->im)
    return;
  gboolean marked = b->im->preedit && *b->im->preedit;
  if (preedit) {
    if (!marked)
      g_signal_emit_by_name(b->im, "preedit-started");
    g_free(b->im->preedit);
    b->im->preedit = g_strdup(text);
    g_signal_emit_by_name(b->im, "preedit-changed");
  } else {
    if (marked) {
      g_clear_pointer(&b->im->preedit, g_free);
      g_signal_emit_by_name(b->im, "preedit-changed");
      g_signal_emit_by_name(b->im, "preedit-finished");
    }
    if (*text)
      g_signal_emit_by_name(b->im, "committed", text);
  }
}
void bw_set_clipboard(BrowserView *b, const char *text) {
  if (defer_input(b, 2, 0, text, 0, 0))
    return;
  b->display->setting_clipboard = TRUE;
  WPEClipboard *c = display_clipboard(WPE_DISPLAY(b->display));
  WPEClipboardContent *content = wpe_clipboard_content_new();
  wpe_clipboard_content_set_text(content, text);
  wpe_clipboard_set_content(c, content);
  b->display->setting_clipboard = FALSE;
  wpe_clipboard_content_unref(content);
  b->clipboard_count = wpe_clipboard_get_change_count(c);
}
char *bw_take_clipboard(BrowserView *b) {
  WPEClipboard *c = display_clipboard(WPE_DISPLAY(b->display));
  gint64 count = wpe_clipboard_get_change_count(c);
  if (count == b->clipboard_count)
    return NULL;
  b->clipboard_count = count;
  GBytes *bytes = wpe_clipboard_read_bytes(c, "text/plain");
  if (!bytes)
    return NULL;
  gsize length;
  const char *data = g_bytes_get_data(bytes, &length);
  char *text = g_strndup(data, length);
  g_bytes_unref(bytes);
  return text;
}

static void evaluated(GObject *source, GAsyncResult *result, gpointer data) {
  (void)data;
  GError *error = NULL;
  JSCValue *value = webkit_web_view_evaluate_javascript_finish(
      WEBKIT_WEB_VIEW(source), result, &error);
  char *text = value ? jsc_value_to_string(value)
                     : g_strdup(error ? error->message : "Evaluation failed");
  g_object_set_data_full(source, "gpui-diagnostic", text, g_free);
  g_clear_object(&value);
  g_clear_error(&error);
}
void bw_evaluate(BrowserView *b, const char *script) {
  trace_native("evaluate view=%p bytes=%zu", b, strlen(script));
  if (!b->javascript) {
    g_object_set_data_full(G_OBJECT(b->web), "gpui-diagnostic",
                           g_strdup("JavaScript is disabled"), g_free);
    bw_wake();
    return;
  }
  webkit_web_view_evaluate_javascript(b->web, script, -1, NULL, NULL, NULL,
                                      evaluated, NULL);
}
char *bw_take_evaluation(BrowserView *b) {
  return g_object_steal_data(G_OBJECT(b->web), "gpui-diagnostic");
}

int bw_shutdown(void) {
  if (live_views)
    return 0;
  if (resource_policy) {
    ResourcePolicyJob *job = resource_policy;
    resource_policy = NULL;
    job->retired = TRUE;
    if (job->timeout) {
      g_source_remove(job->timeout);
      job->timeout = 0;
    }
    g_cancellable_cancel(job->cancel);
    /* Shutdown has no remaining native views. Give the cancelled compiler a
     * bounded chance to finish cleanup; a late callback still owns its job,
     * cannot access retired views, and will remove its constant-only store. */
    int64_t deadline = g_get_monotonic_time() + 100000;
    while (job->pending && g_get_monotonic_time() < deadline) {
      g_main_context_iteration(NULL, FALSE);
      if (job->pending)
        g_usleep(1000);
    }
    policy_unref(job);
  }
  for (unsigned i = 0; i < G_N_ELEMENTS(shared_displays); i++) {
    DemoDisplay *d = shared_displays[i];
    if (d && d->deferred_source) {
      g_source_remove(d->deferred_source);
      d->deferred_source = 0;
    }
    if (d)
      g_queue_clear_full(&d->deferred, deferred_free);
    g_clear_object(&shared_displays[i]);
  }
  g_clear_pointer(&live_registry, g_ptr_array_unref);
  clipboard_scripts_used = FALSE;
  return 1;
}

void bw_wake(void) { g_main_context_wakeup(g_main_context_default()); }
int bw_poll_prepare(BwPollFd *fds, int capacity, int *timeout_ms) {
  GMainContext *context = g_main_context_default();
  if (!g_main_context_acquire(context)) {
    *timeout_ms = -1;
    return -1;
  }
  gint priority = 0;
  gboolean ready = g_main_context_prepare(context, &priority);
  // GPollFD intentionally has the same public fd/events/revents shape.
  int count = g_main_context_query(context, priority, timeout_ms,
                                   (GPollFD *)fds, capacity);
  if (ready)
    *timeout_ms = 0;
  g_main_context_release(context);
  return count;
}
void bw_metrics(BrowserView *b, BwMetrics *metrics) { *metrics = b->metrics; }
void bw_live_metrics(BwLiveMetrics *result) {
  *result = (BwLiveMetrics){0};
  result->views = live_views;
  result->policy_jobs = resource_policy_jobs;
  result->sources += resource_policy && !!resource_policy->timeout;
  for (unsigned i = 0; i < G_N_ELEMENTS(shared_displays); i++) {
    DemoDisplay *d = shared_displays[i];
    if (!d)
      continue;
    result->displays++;
    result->sources += !!d->selection_timeout + !!d->deferred_source;
    result->sources += g_queue_get_length(&d->copies_pending);
  }
  if (!live_registry)
    return;
  for (guint i = 0; i < live_registry->len; i++) {
    BrowserView *b = g_ptr_array_index(live_registry, i);
    result->policy_waiters += b->restricted && !b->policy_ready && !b->policy_failed;
    result->imports += b->imports->len;
    result->pending_frames += g_queue_get_length(&b->frames_pending);
    result->sources += !!b->completion_source;
    for (GList *link = b->frames_pending.head; link; link = link->next) {
      Frame *frame = link->data;
      result->sources += !!frame->source + !!frame->timeout;
    }
    GHashTableIter it;
    gpointer value;
    g_hash_table_iter_init(&it, b->resource_waits);
    while (g_hash_table_iter_next(&it, NULL, &value)) {
      ResourceWait *wait = value;
      result->resource_waits++;
      result->sources += !!wait->timeout;
    }
    g_hash_table_iter_init(&it, b->resources);
    while (g_hash_table_iter_next(&it, NULL, &value))
      result->resource_bytes += g_bytes_get_size(((Resource *)value)->bytes);
    g_hash_table_iter_init(&it, b->documents);
    while (g_hash_table_iter_next(&it, NULL, &value))
      result->document_bytes += strlen(value);
  }
}
double bw_get_zoom(BrowserView *b) {
  return webkit_web_view_get_zoom_level(b->web);
}
void bw_find(BrowserView *b, const char *text, int case_sensitive) {
  if (!text || !*text) {
    bw_find_clear(b);
    return;
  }
  b->find_pending = TRUE;
  b->find_matches = 0;
  unsigned flags = WEBKIT_FIND_OPTIONS_WRAP_AROUND |
                   (case_sensitive ? 0 : WEBKIT_FIND_OPTIONS_CASE_INSENSITIVE);
  webkit_find_controller_search(webkit_web_view_get_find_controller(b->web),
                                text, flags, 100000);
  bw_wake();
}
void bw_find_next(BrowserView *b, int backwards) {
  if (backwards)
    webkit_find_controller_search_previous(
        webkit_web_view_get_find_controller(b->web));
  else
    webkit_find_controller_search_next(
        webkit_web_view_get_find_controller(b->web));
  bw_wake();
}
void bw_find_clear(BrowserView *b) {
  b->find_matches = 0;
  b->find_pending = FALSE;
  webkit_find_controller_search_finish(
      webkit_web_view_get_find_controller(b->web));
  bw_wake();
}
unsigned bw_find_matches(BrowserView *b) { return b->find_matches; }
int bw_find_pending(BrowserView *b) { return b->find_pending; }
char *bw_runtime_info(void) {
  Dl_info info = {0};
  dladdr((void *)webkit_get_major_version, &info);
  return g_strdup_printf("%u.%u.%u\n%s", webkit_get_major_version(),
                         webkit_get_minor_version(), webkit_get_micro_version(),
                         info.dli_fname ? info.dli_fname : "unknown");
}

void bw_clip(BrowserView *b, double x, double y, double width, double height) {
  if (!isfinite(x) || !isfinite(y) || !isfinite(width) || !isfinite(height) ||
      x < 0 || y < 0 || width < 0 || height < 0 || x + width > b->width + 1 ||
      y + height > b->height + 1) {
    set_error(b, "Invalid viewport clip");
    return;
  }
  if (b->clip_set && b->clip_x == x && b->clip_y == y &&
      b->clip_width == width && b->clip_height == height)
    return;
  trace_native("clip view=%p rect=%.3f,%.3f %.3fx%.3f", b, x, y, width, height);
  b->clip_set = TRUE;
  b->clip_x = x;
  b->clip_y = y;
  b->clip_width = width;
  b->clip_height = height;
  wl_egl_window_resize(b->window, MAX(1, (int)ceil(width * b->scale)),
                       MAX(1, (int)ceil(height * b->scale)), 0, 0);
  if (eglMakeCurrent(b->egl, b->surface, b->surface, b->gl))
    present_snapshot(b);
  bw_wake();
}

int bw_sample_pixel(BrowserView *b, int x, int y, unsigned char *rgba) {
  if (!rgba || !b->snapshot_valid ||
      b->snapshot_document_id != b->document_id || x < 0 || y < 0 ||
      x >= b->snapshot_width || y >= b->snapshot_height || !b->snapshot_fbo)
    return 0;
  if (!eglMakeCurrent(b->egl, b->surface, b->surface, b->gl))
    return 0;
  glBindFramebuffer(GL_FRAMEBUFFER, b->snapshot_fbo);
  // Explicit diagnostic readback: this may synchronize; never used by normal
  // presentation.
  glReadPixels(x, b->snapshot_height - 1 - y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE,
               rgba);
  GLenum error = glGetError();
  glBindFramebuffer(GL_FRAMEBUFFER, 0);
  return error == GL_NO_ERROR;
}
int64_t bw_monotonic_us(void) { return g_get_monotonic_time(); }
