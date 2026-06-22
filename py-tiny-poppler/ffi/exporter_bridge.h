#ifndef TINY_POPPLER_EXPORTER_BRIDGE_H
#define TINY_POPPLER_EXPORTER_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#include "splash_bridge.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef enum nt_image_export_match_mode
{
    NTIMAGE_EXPORT_MATCH_BY_REF = 0,
    NTIMAGE_EXPORT_MATCH_BY_OCCURRENCE = 1,
} nt_image_export_match_mode_t;

typedef enum nt_image_export_type
{
    NTIMAGE_EXPORT_TYPE_IMAGE = 0,
    NTIMAGE_EXPORT_TYPE_STENCIL = 1,
    NTIMAGE_EXPORT_TYPE_MASK = 2,
    NTIMAGE_EXPORT_TYPE_SOFT_MASK = 3,
} nt_image_export_type_t;

typedef enum nt_image_export_format
{
    NTIMAGE_EXPORT_FORMAT_UNKNOWN = 0,
    NTIMAGE_EXPORT_FORMAT_RGB = 1,
    NTIMAGE_EXPORT_FORMAT_RGB48 = 2,
    NTIMAGE_EXPORT_FORMAT_GRAY = 3,
    NTIMAGE_EXPORT_FORMAT_MONOCHROME = 4,
    NTIMAGE_EXPORT_FORMAT_CMYK = 5,
} nt_image_export_format_t;

typedef enum nt_image_export_extension
{
    NTIMAGE_EXPORT_EXTENSION_JPEG = 0,
    NTIMAGE_EXPORT_EXTENSION_JP2 = 1,
    NTIMAGE_EXPORT_EXTENSION_JBIG2 = 2,
    NTIMAGE_EXPORT_EXTENSION_CCITT = 3,
    NTIMAGE_EXPORT_EXTENSION_CCITT_TIFF = 4,
    NTIMAGE_EXPORT_EXTENSION_PNG = 5,
    NTIMAGE_EXPORT_EXTENSION_TIFF = 6,
    NTIMAGE_EXPORT_EXTENSION_PNM = 7,
} nt_image_export_extension_t;

typedef struct nt_image_export_params {
    uint32_t page_index;
    nt_image_export_match_mode_t match_mode;
    nt_image_export_type_t target_type;
    int32_t xref_object;
    int32_t xref_generation;
    uint32_t occurrence_index;
} nt_image_export_params_t;

typedef struct nt_image_ccitt_params {
    int32_t encoding;
    int32_t columns;
    int32_t rows;
    int32_t damaged_rows_before_error;
    uint8_t end_of_line;
    uint8_t byte_align;
    uint8_t end_of_block;
    uint8_t black_is_one;
} nt_image_ccitt_params_t;

typedef struct nt_image_export_image {
    uint8_t *data;
    size_t len;
    uint32_t width;
    uint32_t height;
    uint32_t stride;
    uint32_t components;
    uint32_t bits_per_component;
    double width_dpi;
    double height_dpi;
    nt_image_export_format_t format;
    nt_image_export_type_t type;
    nt_image_export_extension_t extension;
    uint8_t has_jbig2_globals;
    uint8_t *jbig2_globals;
    size_t jbig2_globals_len;
    uint8_t has_ccitt_params;
    nt_image_ccitt_params_t ccitt;
} nt_image_export_image_t;

int ntsplash_exporer_extract_page(
    ntsplash_renderer_t *renderer,          // renderer from internal
    const nt_image_export_params_t *params, // export parameters
    nt_image_export_image_t *out_image,     // output image buffer
    bool describe_only, // This parameter would not actually "extract" anything when true, this will
                        // keep the buffer empty
    char **error_out);

void ntsplash_exporter_free(nt_image_export_image_t *image);

#ifdef __cplusplus
}
#endif

#endif // TINY_POPPLER_EXPORTER_BRIDGE_H
