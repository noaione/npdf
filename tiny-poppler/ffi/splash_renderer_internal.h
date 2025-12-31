#pragma once
#ifndef TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
#define TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H

#include <memory>

class PDFDoc;

struct ntsplash_renderer {
    std::unique_ptr<PDFDoc> doc;
};

void ntsplash_set_error(char **error_out, const std::string &message)
{
    if (!error_out)
    {
        return;
    }
    *error_out = nullptr;
    const size_t len = message.size();
    char *buffer = static_cast<char *>(std::malloc(len + 1));
    if (!buffer)
    {
        return;
    }
    std::memcpy(buffer, message.c_str(), len);
    buffer[len] = '\0';
    *error_out = buffer;
}

std::string ntsplash_stringify_error_code(int error_code)
{
    switch (error_code)
    {
    case errNone:
        return "ok";
    case errOpenFile:
        return "failed to open PDF";
    case errBadCatalog:
        return "invalid PDF catalog";
    case errDamaged:
        return "PDF is damaged and could not be repaired";
    case errEncrypted:
        return "PDF is encrypted and no password was provided";
    case errHighlightFile:
        return "invalid highlight file";
    case errBadPrinter:
        return "invalid printer configuration";
    case errPrinting:
        return "error while printing";
    case errPermission:
        return "operation not permitted by PDF";
    case errBadPageNum:
        return "invalid page number";
    case errFileIO:
        return "file I/O failure";
    case errFileChangedSinceOpen:
        return "PDF changed since open";
    default:
        return "unknown poppler error";
    }
}

#endif // TINY_POPPLER_SPLASH_RENDERER_INTERNAL_H
