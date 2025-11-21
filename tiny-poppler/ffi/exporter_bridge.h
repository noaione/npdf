#ifndef TINY_POPPLER_EXPORTER_BRIDGE_H
#define TINY_POPPLER_EXPORTER_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#include "splash_bridge.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum image_export_match_mode {
	IMAGE_EXPORT_MATCH_BY_REF = 0,
	IMAGE_EXPORT_MATCH_BY_TYPE = 1,
} image_export_match_mode_t;

typedef enum image_export_type {
	IMAGE_EXPORT_TYPE_IMAGE = 0,
	IMAGE_EXPORT_TYPE_STENCIL = 1,
	IMAGE_EXPORT_TYPE_MASK = 2,
	IMAGE_EXPORT_TYPE_SOFT_MASK = 3,
} image_export_type_t;

typedef enum image_export_format {
	IMAGE_EXPORT_FORMAT_UNKNOWN = 0,
	IMAGE_EXPORT_FORMAT_RGB = 1,
	IMAGE_EXPORT_FORMAT_RGB48 = 2,
	IMAGE_EXPORT_FORMAT_GRAY = 3,
	IMAGE_EXPORT_FORMAT_MONOCHROME = 4,
	IMAGE_EXPORT_FORMAT_CMYK = 5,
} image_export_format_t;

typedef enum image_export_extension {
	IMAGE_EXPORT_EXTENSION_JPEG = 0,
	IMAGE_EXPORT_EXTENSION_JP2 = 1,
	IMAGE_EXPORT_EXTENSION_JBIG2 = 2,
	IMAGE_EXPORT_EXTENSION_CCITT = 3,
	IMAGE_EXPORT_EXTENSION_CCITT_TIFF = 4,
	IMAGE_EXPORT_EXTENSION_PNG = 5,
	IMAGE_EXPORT_EXTENSION_TIFF = 6,
	IMAGE_EXPORT_EXTENSION_PNM = 7,
} image_export_extension_t;

typedef struct image_export_params {
	uint32_t page_index;
	image_export_match_mode_t match_mode;
	image_export_type_t target_type;
	int32_t xref_object;
	int32_t xref_generation;
} image_export_params_t;

typedef struct image_export_image {
	uint8_t *data;
	size_t len;
	uint32_t width;
	uint32_t height;
	uint32_t stride;
	uint32_t components;
	uint32_t bits_per_component;
	image_export_format_t format;
	image_export_type_t type;
	image_export_extension_t extension;
} image_export_image_t;

int image_exporter_extract(splash_renderer_t *renderer,
						   const image_export_params_t *params,
						   image_export_image_t *out_image,
						   char **error_out);

void image_exporter_free(image_export_image_t *image);

#ifdef __cplusplus
}
#endif

#endif // TINY_POPPLER_EXPORTER_BRIDGE_H
