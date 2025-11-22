#ifndef TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
#define TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H

#include <memory>
#include <mutex>

class PDFDoc;

struct splash_renderer {
    std::unique_ptr<PDFDoc> doc;
};

// mutexes for globalParams initialization
std::once_flag global_params_init_flag;

void ensure_global_params()
{
    std::call_once(global_params_init_flag, [] {
        globalParams = std::make_unique<GlobalParams>();
        globalParams->setErrQuiet(true);
    });
}

#endif // TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
