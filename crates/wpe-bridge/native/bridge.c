#include "bridge.h"
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>
#include <drm_fourcc.h>
#include <math.h>
#include <poll.h>
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
  WPEDRMDevice *device;
  WPEBufferFormats *formats;
  WPEClipboard *clipboard;
} DemoDisplay;
typedef WPEDisplayClass DemoDisplayClass;
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
G_DEFINE_TYPE(DemoDisplay, demo_display, WPE_TYPE_DISPLAY)
G_DEFINE_TYPE(DemoToplevel, demo_toplevel, WPE_TYPE_TOPLEVEL)
G_DEFINE_TYPE(DemoView, demo_view, WPE_TYPE_VIEW)
G_DEFINE_TYPE(DemoIM, demo_im, WPE_TYPE_INPUT_METHOD_CONTEXT)

/* WPEDisplay owns an isolated device/surfaceless EGLDisplay. Presenters
 * borrow GPUI's Wayland EGLDisplay and never terminate it. Share the engine
 * across tabs, then release it after explicit native close in the quit hook. */
static DemoDisplay *shared_display;
static unsigned live_views;

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
  int width, height, visible, email, registered;
  double scale;
  uint64_t frames;
  char *error, *new_uri, *cursor, *email_html;
  gint64 clipboard_count;
};

static void set_error(BrowserView *b, const char *message) {
  g_free(b->error);
  b->error = g_strdup(message);
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
    self->clipboard = wpe_clipboard_new(d);
  return self->clipboard;
}
static WPEInputMethodContext *display_im(WPEDisplay *d, WPEView *v) {
  (void)d;
  DemoIM *im = g_object_new(demo_im_get_type(), "view", v, NULL);
  BrowserView *b = ((DemoView *)v)->owner;
  if (b) {
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
  /* EGLDisplay can be shared with other tabs: never eglTerminate here. */
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
static gboolean render_buffer(WPEView *v, WPEBuffer *buffer,
                              const WPERectangle *damage, guint count,
                              GError **error) {
  (void)damage;
  (void)count;
  BrowserView *b = ((DemoView *)v)->owner;
  if (!b) {
    g_set_error_literal(error, WPE_VIEW_ERROR, WPE_VIEW_ERROR_RENDER_FAILED,
                        "View is shutting down");
    return FALSE;
  }
  if (!eglMakeCurrent(b->egl, b->surface, b->surface, b->gl))
    goto failed;
  int fence = wpe_buffer_take_rendering_fence(buffer);
  if (fence >= 0) {
    /* Conservative initial synchronization. Never advertise explicit sync.
     * A bounded wait avoids an indefinitely blocked UI on a dead producer. */
    struct pollfd p = {fence, POLLIN, 0};
    int ready = poll(&p, 1, 100);
    close(fence);
    if (ready <= 0) {
      set_error(b, "Timed out waiting for WPE rendering fence");
      goto failed;
    }
  }
  int width = wpe_buffer_get_width(buffer),
      height = wpe_buffer_get_height(buffer);
  glBindTexture(GL_TEXTURE_2D, b->texture);
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
    // The GL texture retains the image storage after destroying the EGL handle.
    destroy_image(b->egl, image);
  } else if (WPE_IS_BUFFER_SHM(buffer)) {
    WPEBufferSHM *shm = WPE_BUFFER_SHM(buffer);
    GBytes *bytes = wpe_buffer_shm_get_data(shm);
    const guint8 *src = g_bytes_get_data(bytes, NULL);
    int stride = wpe_buffer_shm_get_stride(shm);
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
  if (b->visible && width == (int)ceil(b->width * b->scale) &&
      height == (int)ceil(b->height * b->scale)) {
    static const GLfloat vertices[] = {-1, -1, 0, 1, 1, -1, 1, 1,
                                       -1, 1,  0, 0, 1, 1,  1, 0};
    glViewport(0, 0, width, height);
    glUseProgram(b->program);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 4 * sizeof(GLfloat),
                          vertices);
    glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 4 * sizeof(GLfloat),
                          vertices + 2);
    glEnableVertexAttribArray(0);
    glEnableVertexAttribArray(1);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    /* glFinish proves source sampling has ended before release. This is a
     * deliberately conservative prototype path, not a performance claim. */
    glFinish();
    if (!eglSwapBuffers(b->egl, b->surface))
      goto failed;
    b->frames++;
  } else
    glFinish();
  wpe_view_buffer_rendered(v, buffer);
  wpe_view_buffer_released(v, buffer);
  return TRUE;
failed:
  if (!b->error)
    set_error(b, "EGL presentation failed (check renderer/DMA-BUF support)");
  if (error && !*error)
    g_set_error_literal(error, WPE_VIEW_ERROR, WPE_VIEW_ERROR_RENDER_FAILED,
                        b->error);
  return FALSE;
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
  if (!g_error_matches(e, WEBKIT_NETWORK_ERROR, WEBKIT_NETWORK_ERROR_CANCELLED))
    set_error(b, e->message);
  return TRUE;
}
static void terminated(WebKitWebView *v,
                       WebKitWebProcessTerminationReason reason,
                       BrowserView *b) {
  (void)v;
  (void)reason;
  set_error(b, "Web process terminated. Reload to recover.");
}
static gboolean allowed_uri(const char *uri) {
  return uri &&
         (g_str_has_prefix(uri, "https://") ||
          g_str_has_prefix(uri, "http://") || g_str_has_prefix(uri, "demo://"));
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
  if (type == WEBKIT_POLICY_DECISION_TYPE_NEW_WINDOW_ACTION) {
    if (allowed_uri(uri) && webkit_navigation_action_is_user_gesture(action)) {
      g_free(b->new_uri);
      b->new_uri = g_strdup(uri);
    }
    webkit_policy_decision_ignore(decision);
    return TRUE;
  }
  if (b->email && !strcmp(uri, "message://body/"))
    return FALSE;
  if (b->email || !allowed_uri(uri)) {
    if (b->email && allowed_uri(uri) &&
        webkit_navigation_action_is_user_gesture(action)) {
      g_free(b->new_uri);
      b->new_uri = g_strdup(uri);
    }
    webkit_policy_decision_ignore(decision);
    return TRUE;
  }
  return FALSE;
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
  (void)d;
  (void)data;
  return TRUE;
}

/* Trusted, compiled-in fixtures. No arbitrary file URLs are exposed. */
#include "fixtures.h"
static void fixture_request(WebKitURISchemeRequest *r, gpointer data) {
  BrowserView *b = g_object_get_data(
      G_OBJECT(webkit_uri_scheme_request_get_web_view(r)), "gpui-owner");
  (void)data;
  const char *uri = webkit_uri_scheme_request_get_uri(r);
  const char *html = NULL;
  if (!strcmp(uri, "message://body/") && b && b->email)
    html = b->email_html;
  else if (!strcmp(uri, "demo://editing/"))
    html = fixture_editing;
  else if (!strcmp(uri, "demo://second/"))
    html = fixture_second;
  if (!html) {
    GError *e = g_error_new_literal(G_IO_ERROR, G_IO_ERROR_NOT_FOUND,
                                    "Unknown demo resource");
    webkit_uri_scheme_request_finish_error(r, e);
    g_error_free(e);
    return;
  }
  GInputStream *s =
      g_memory_input_stream_new_from_data(g_strdup(html), strlen(html), g_free);
  webkit_uri_scheme_request_finish(r, s, strlen(html), "text/html");
  g_object_unref(s);
}

BrowserView *bw_create(void *native_display, void *native_surface,
                       char **error) {
  BrowserView *b = g_new0(BrowserView, 1);
  b->surface = EGL_NO_SURFACE;
  b->gl = EGL_NO_CONTEXT;
  b->width = b->height = 1;
  b->scale = 1;
  b->display = shared_display ? g_object_ref(shared_display)
                              : g_object_new(demo_display_get_type(), NULL);
  b->display->owner = b;
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
  EGLConfig config;
  EGLint n;
  if (!eglChooseConfig(b->egl, attrs, &config, 1, &n) || !n)
    goto failed;
  const EGLint ctx[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
  b->gl = eglCreateContext(b->egl, config, EGL_NO_CONTEXT, ctx);
  b->window = wl_egl_window_create(native_surface, 1, 1);
  if (!b->window)
    goto failed;
  b->surface = eglCreateWindowSurface(b->egl, config,
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
  glGenTextures(1, &b->texture);
  glBindTexture(GL_TEXTURE_2D, b->texture);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
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
    if (!shared_display)
      shared_display = g_object_ref(b->display);
  }
  wpe_display_set_available_input_devices(
      WPE_DISPLAY(b->display),
      WPE_AVAILABLE_INPUT_DEVICE_MOUSE | WPE_AVAILABLE_INPUT_DEVICE_KEYBOARD);
  b->context = webkit_web_context_new();
  webkit_web_context_register_uri_scheme(b->context, "demo", fixture_request,
                                         NULL, NULL);
  webkit_web_context_register_uri_scheme(b->context, "message", fixture_request,
                                         NULL, NULL);
  b->session = webkit_network_session_new_ephemeral();
  g_signal_connect(b->session, "download-started", G_CALLBACK(deny_download),
                   NULL);
  b->web =
      g_object_new(WEBKIT_TYPE_WEB_VIEW, "display", b->display, "web-context",
                   b->context, "network-session", b->session, NULL);
  b->display->owner = NULL;
  g_object_set_data(G_OBJECT(b->web), "gpui-owner", b);
  b->view = webkit_web_view_get_wpe_view(b->web);
  ((DemoView *)b->view)->owner = b;
  b->toplevel = wpe_display_create_toplevel(WPE_DISPLAY(b->display), 1);
  wpe_view_set_toplevel(b->view, b->toplevel);
  g_signal_connect(b->web, "load-failed", G_CALLBACK(load_failed), b);
  g_signal_connect(b->web, "web-process-terminated", G_CALLBACK(terminated), b);
  g_signal_connect(b->web, "decide-policy", G_CALLBACK(decide_policy), b);
  g_signal_connect(b->web, "permission-request", G_CALLBACK(deny_permission),
                   NULL);
  g_signal_connect(b->web, "script-dialog", G_CALLBACK(deny_dialog), NULL);
  b->clipboard_count = wpe_clipboard_get_change_count(
      display_clipboard(WPE_DISPLAY(b->display)));
  b->registered = 1;
  live_views++;
  return b;
failed:
  if (error)
    *error = g_strdup_printf(
        "Cannot initialize WPE/EGL presenter (EGL error 0x%x)", eglGetError());
  bw_destroy(b);
  return NULL;
}
void bw_destroy(BrowserView *b) {
  if (!b)
    return;
  if (b->web) {
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
  if (b->gl != EGL_NO_CONTEXT && b->surface != EGL_NO_SURFACE &&
      eglMakeCurrent(b->egl, b->surface, b->surface, b->gl)) {
    glFinish();
    glDeleteTextures(1, &b->texture);
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
  if (b->registered)
    live_views--;
  g_clear_object(&b->display);
  g_free(b->error);
  g_free(b->new_uri);
  g_free(b->cursor);
  g_free(b->email_html);
  g_free(b);
}
void bw_free(void *p) { g_free(p); }
void bw_pump(void) {
  for (int i = 0; i < 32 && g_main_context_pending(NULL); i++)
    g_main_context_iteration(NULL, FALSE);
}
void bw_geometry(BrowserView *b, int w, int h, double scale, int visible) {
  visible = visible && w > 0 && h > 0;
  if (b->width != w || b->height != h || b->scale != scale) {
    b->width = w;
    b->height = h;
    b->scale = scale;
    wl_egl_window_resize(b->window, MAX(1, (int)ceil(w * scale)),
                         MAX(1, (int)ceil(h * scale)), 0, 0);
    wpe_toplevel_scale_changed(b->toplevel, scale);
    wpe_toplevel_resized(b->toplevel, MAX(w, 1), MAX(h, 1));
    wpe_view_resized(b->view, MAX(w, 1), MAX(h, 1));
  }
  if (b->visible != visible) {
    b->visible = visible;
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
  wpe_toplevel_state_changed(b->toplevel,
                             focused ? WPE_TOPLEVEL_STATE_ACTIVE : 0);
  if (focused)
    wpe_view_focus_in(b->view);
  else
    wpe_view_focus_out(b->view);
}
void bw_load(BrowserView *b, const char *uri) {
  if (!allowed_uri(uri)) {
    set_error(b, "Only HTTP(S) and built-in demo URIs are allowed");
    return;
  }
  b->email = 0;
  g_clear_pointer(&b->error, g_free);
  webkit_settings_set_enable_javascript(webkit_web_view_get_settings(b->web),
                                        TRUE);
  webkit_web_view_load_uri(b->web, uri);
}
void bw_load_html(BrowserView *b, const char *html, const char *base,
                  int email) {
  b->email = email;
  g_clear_pointer(&b->error, g_free);
  webkit_settings_set_enable_javascript(webkit_web_view_get_settings(b->web),
                                        !email);
  if (email) {
    g_free(b->email_html);
    b->email_html = g_strdup(html);
    webkit_web_view_load_uri(b->web, "message://body/");
  } else
    webkit_web_view_load_html(b->web, html, base);
}
void bw_action(BrowserView *b, int action) {
  switch (action) {
  case 0:
    webkit_web_view_go_back(b->web);
    break;
  case 1:
    webkit_web_view_go_forward(b->web);
    break;
  case 2:
    g_clear_pointer(&b->error, g_free);
    webkit_web_view_reload(b->web);
    break;
  case 3:
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
  webkit_web_view_set_zoom_level(b->web, zoom);
}
const char *bw_uri(BrowserView *b) { return webkit_web_view_get_uri(b->web); }
const char *bw_title(BrowserView *b) {
  return webkit_web_view_get_title(b->web);
}
const char *bw_error(BrowserView *b) { return b->error; }
char *bw_take_new_uri(BrowserView *b) { return g_steal_pointer(&b->new_uri); }
int bw_state(BrowserView *b) {
  return webkit_web_view_is_loading(b->web) |
         (webkit_web_view_can_go_back(b->web) << 1) |
         (webkit_web_view_can_go_forward(b->web) << 2);
}
uint64_t bw_frames(BrowserView *b) { return b->frames; }
const char *bw_cursor(BrowserView *b) { return b->cursor; }
void bw_pointer(BrowserView *b, int kind, double x, double y, int button,
                uint32_t mods, int count) {
  WPEEvent *e;
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
    wpe_view_event(b->view, e);
    wpe_event_unref(e);
  }
}
void bw_scroll(BrowserView *b, double x, double y, double dx, double dy,
               uint32_t mods) {
  WPEEvent *e =
      wpe_event_scroll_new(b->view, WPE_INPUT_SOURCE_TOUCHPAD, event_time(),
                           mods, dx, dy, TRUE, dx == 0 && dy == 0, x, y);
  if (e) {
    wpe_view_event(b->view, e);
    wpe_event_unref(e);
  }
}
void bw_key(BrowserView *b, const char *key, uint32_t mods, int down) {
  xkb_keysym_t sym = xkb_keysym_from_name(key, XKB_KEYSYM_CASE_INSENSITIVE);
  if (sym == XKB_KEY_NoSymbol && g_utf8_strlen(key, -1) == 1)
    sym = xkb_utf32_to_keysym(g_utf8_get_char(key));
  if (sym == XKB_KEY_NoSymbol)
    return;
  WPEEvent *e = wpe_event_keyboard_new(
      down ? WPE_EVENT_KEYBOARD_KEY_DOWN : WPE_EVENT_KEYBOARD_KEY_UP, b->view,
      WPE_INPUT_SOURCE_KEYBOARD, event_time(), mods, 0, sym);
  if (e) {
    wpe_view_event(b->view, e);
    wpe_event_unref(e);
  }
}
void bw_text(BrowserView *b, const char *text, int preedit) {
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
  WPEClipboard *c = display_clipboard(WPE_DISPLAY(b->display));
  WPEClipboardContent *content = wpe_clipboard_content_new();
  wpe_clipboard_content_set_text(content, text);
  wpe_clipboard_set_content(c, content);
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
  webkit_web_view_evaluate_javascript(b->web, script, -1, NULL, NULL, NULL,
                                      evaluated, NULL);
}
char *bw_take_evaluation(BrowserView *b) {
  return g_object_steal_data(G_OBJECT(b->web), "gpui-diagnostic");
}

int bw_shutdown(void) {
  if (live_views)
    return 0;
  g_clear_object(&shared_display);
  return 1;
}
