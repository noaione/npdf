#ifndef TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
#define TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H

#include <memory>
#include <mutex>
#include "GlobalParams.h"

class PDFDoc;

struct splash_renderer {
    std::unique_ptr<PDFDoc> doc;
};

void ensure_global_params()
{
    if (!globalParams) {
        globalParams = std::make_unique<GlobalParams>();
        globalParams->setErrQuiet(true);
    }
}

#endif // TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
