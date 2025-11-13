#ifndef TINY_POPPLER_SPLASH_BRIDGE_H
#define TINY_POPPLER_SPLASH_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct splash_renderer splash_renderer_t;

typedef enum splash_color_mode {
    SPLASH_COLOR_MODE_MONO1 = 0,
    SPLASH_COLOR_MODE_MONO8 = 1,
    SPLASH_COLOR_MODE_RGB8 = 2,
    SPLASH_COLOR_MODE_BGR8 = 3,
    SPLASH_COLOR_MODE_XBGR8 = 4,
    SPLASH_COLOR_MODE_CMYK8 = 5,
    SPLASH_COLOR_MODE_DEVICEN8 = 6,
} splash_color_mode_t;

typedef enum splash_image_colorspace {
    SPLASH_IMAGE_COLORSPACE_UNKNOWN = 0,
    SPLASH_IMAGE_COLORSPACE_DEVICE_GRAY = 1,
    SPLASH_IMAGE_COLORSPACE_DEVICE_RGB = 2,
    SPLASH_IMAGE_COLORSPACE_DEVICE_CMYK = 3,
    SPLASH_IMAGE_COLORSPACE_LAB = 4,
    SPLASH_IMAGE_COLORSPACE_ICC = 5,
    SPLASH_IMAGE_COLORSPACE_INDEXED = 6,
    SPLASH_IMAGE_COLORSPACE_PATTERN = 7,
    SPLASH_IMAGE_COLORSPACE_SEPARATION = 8,
    SPLASH_IMAGE_COLORSPACE_DEVICEN = 9,
    SPLASH_IMAGE_COLORSPACE_OTHER = 10,
} splash_image_colorspace_t;

typedef struct splash_image {
    uint8_t *data;
    size_t len;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint32_t components;
    splash_color_mode_t color_mode;
    uint32_t bits_per_component;
} splash_image_t;

typedef struct splash_image_info {
    uint32_t width;
    uint32_t height;
    uint32_t components;
    uint32_t bits_per_component;
    int32_t xref_object;
    int32_t xref_generation;
    splash_image_colorspace_t colorspace;
} splash_image_info_t;

int splash_renderer_create(const char *path, splash_renderer_t **out_renderer, char **error_out);
void splash_renderer_destroy(splash_renderer_t *renderer);

int splash_renderer_page_count(splash_renderer_t *renderer, uint32_t *out_count, char **error_out);

int splash_renderer_render_page(splash_renderer_t *renderer,
                                uint32_t page_index,
                                double dpi,
                                splash_color_mode_t color_mode,
                                splash_image_t *out_image,
                                char **error_out);

int splash_renderer_collect_images(splash_renderer_t *renderer,
                                   splash_image_info_t **out_images,
                                   size_t *out_len,
                                   char **error_out);

void splash_renderer_free_image(splash_image_t *image);
void splash_renderer_free_cstr(char *message);
void splash_renderer_free_image_info(splash_image_info_t *images);

#ifdef __cplusplus
}
#endif

#endif // TINY_POPPLER_SPLASH_BRIDGE_H
