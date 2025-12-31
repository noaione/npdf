#pragma once
#ifndef TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
#define TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H

#include <memory>
#include <string>

class PDFDoc;

struct ntsplash_renderer {
    std::unique_ptr<PDFDoc> doc;
};

void ntsplash_set_error(char **error_out, const std::string &message);
std::string ntsplash_stringify_error_code(int error_code);

#endif // TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
