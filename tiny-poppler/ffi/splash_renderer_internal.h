#ifndef TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
#define TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H

#include <memory>

class PDFDoc;

struct splash_renderer {
    std::unique_ptr<PDFDoc> doc;
};

#endif // TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
