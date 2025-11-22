//========================================================================
//
// ImageOutputDev.cc
//
// Copyright 1998-2003 Glyph & Cog, LLC
//
//========================================================================

//========================================================================
//
//========================================================================
//
// Modified under the Poppler project - http://poppler.freedesktop.org
// And modified again for tiny-poppler/npdf project + renamed into
// image_exporter.cc - https://github.com/noaione/npdf
//
// All changes made under the Poppler project to this file are licensed
// under GPL version 2 or later
//
// Copyright (C) 2005, 2007, 2011, 2018, 2019, 2021, 2022, 2025 Albert Astals Cid <aacid@kde.org>
// Copyright (C) 2006 Rainer Keller <class321@gmx.de>
// Copyright (C) 2008 Timothy Lee <timothy.lee@siriushk.com>
// Copyright (C) 2008 Vasile Gaburici <gaburici@cs.umd.edu>
// Copyright (C) 2009 Carlos Garcia Campos <carlosgc@gnome.org>
// Copyright (C) 2009 William Bader <williambader@hotmail.com>
// Copyright (C) 2010 Jakob Voss <jakob.voss@gbv.de>
// Copyright (C) 2012, 2013, 2017, 2018 Adrian Johnson <ajohnson@redneon.com>
// Copyright (C) 2013 Thomas Fischer <fischer@unix-ag.uni-kl.de>
// Copyright (C) 2013 Hib Eris <hib@hiberis.nl>
// Copyright (C) 2017 Caolán McNamara <caolanm@redhat.com>
// Copyright (C) 2018 Andreas Gruenbacher <agruenba@redhat.com>
// Copyright (C) 2020 mrbax <12640-mrbax@users.noreply.gitlab.freedesktop.org>
// Copyright (C) 2024 Fernando Herrera <fherrera@onirica.com>
// Copyright (C) 2024 Sebastian J. Bronner <waschtl@sbronner.com>
// Copyright (C) 2025 g10 Code GmbH, Author: Sune Stolborg Vuorela <sune@vuorela.dk>
// Copyright (C) 2025 noaione <noaione@n4o.xyz>
//
//========================================================================

#include <poppler-config.h>

#include <cstddef>
#include <cstdio>
#include <cstring>
#include <vector>

#include "Error.h"
#include "GfxState.h"
#include "JBIG2Stream.h"
#include "Stream.h"
#include "goo/gmem.h"
#include "image_exporter.h"

namespace {

ImageOutputDev::ImageFormat determineFormat(GfxImageColorMap *colorMap)
{
    if (!colorMap) {
        return ImageOutputDev::imgMonochrome;
    }

    const int comps = colorMap->getNumPixelComps();
    const int bits = colorMap->getBits();

    if (comps == 4) {
        return ImageOutputDev::imgCMYK;
    }

    if (comps == 3) {
        return bits > 8 ? ImageOutputDev::imgRGB48 : ImageOutputDev::imgRGB;
    }

    if (comps == 1) {
        return bits == 1 ? ImageOutputDev::imgMonochrome : ImageOutputDev::imgGray;
    }

    return ImageOutputDev::imgRGB;
}

ImageOutputDev::ImageExtension defaultExtension(ImageOutputDev::ImageFormat format)
{
    return format == ImageOutputDev::imgCMYK ? ImageOutputDev::extTiff : ImageOutputDev::extPng;
}

constexpr double kDefaultImageDPI = 72.0;

} // namespace

ImageOutputDev::ImageOutputDev(ImageOutput *outputBufferA,
                               const Ref &targetRefA,
                               bool matchRef,
                               ImageType targetType,
                               bool matchOccurrence,
                               uint32_t occurrenceIndex)
    : targetRef(targetRefA)
    , matchByRef(matchRef)
    , matchByOccurrence(matchOccurrence)
    , requestedType(targetType)
    , targetOccurrence(occurrenceIndex)
    , seenOccurrences(0)
    , outputBuffer(outputBufferA)
    , errorCode(0)
{
}

ImageOutputDev::~ImageOutputDev() = default;

long ImageOutputDev::getInlineImageLength(Stream *str, int width, int height, GfxImageColorMap *colorMap)
{
    long len = 0;

    if (colorMap) {
        ImageStream imgStr(str, width, colorMap->getNumPixelComps(), colorMap->getBits());
        if (!imgStr.reset()) {
            imgStr.close();
            return 0;
        }
        for (int y = 0; y < height; ++y) {
            imgStr.getLine();
        }
        imgStr.close();
    } else {
        if (!str->reset()) {
            return 0;
        }
        for (int y = 0; y < height; ++y) {
            const int size = (width + 7) / 8;
            for (int x = 0; x < size; ++x) {
                str->getChar();
            }
        }
    }

    auto *embedStr = static_cast<EmbedStream *>(str->getBaseStream());
    embedStr->rewind();
    while (embedStr->getChar() != EOF) {
        ++len;
    }
    embedStr->restore();

    return len;
}

void ImageOutputDev::storeResult(const std::vector<uint8_t> &buffer,
                                 ImageFormat format,
                                 ImageExtension ext,
                                 ImageType type,
                                 uint32_t width,
                                 uint32_t height,
                                 uint32_t stride,
                                 uint32_t components,
                                 uint32_t bitsPerComponent,
                                 double widthDPI,
                                 double heightDPI,
                                 const std::vector<uint8_t> *jbig2Globals,
                                 const CcittParams *ccittParams)
{
    if (!outputBuffer) {
        return;
    }

    uint8_t *payload = nullptr;
    if (!buffer.empty()) {
        payload = static_cast<uint8_t *>(gmalloc(buffer.size()));
        memcpy(payload, buffer.data(), buffer.size());
    }

    uint8_t *globalsPayload = nullptr;
    size_t globalsLen = 0;
    if (jbig2Globals && !jbig2Globals->empty()) {
        globalsLen = jbig2Globals->size();
        globalsPayload = static_cast<uint8_t *>(gmalloc(globalsLen));
        memcpy(globalsPayload, jbig2Globals->data(), globalsLen);
    }

    outputBuffer->data = payload;
    outputBuffer->len = buffer.size();
    outputBuffer->width = width;
    outputBuffer->height = height;
    outputBuffer->stride = stride;
    outputBuffer->components = components;
    outputBuffer->bits_per_component = bitsPerComponent;
    outputBuffer->wDPI = widthDPI;
    outputBuffer->hDPI = heightDPI;
    outputBuffer->format = format;
    outputBuffer->type = type;
    outputBuffer->extension = ext;
    outputBuffer->jbig2Globals = globalsPayload;
    outputBuffer->jbig2GlobalsLen = globalsLen;
    if (ccittParams) {
        outputBuffer->hasCcittParams = true;
        outputBuffer->ccittParams = *ccittParams;
    } else {
        outputBuffer->hasCcittParams = false;
        outputBuffer->ccittParams = {};
    }
    captured = true;
}

bool ImageOutputDev::matchesTarget(Object *ref, bool inlineImg, ImageType imageType) const
{
    if (captured || !outputBuffer) {
        return false;
    }

    if (matchByRef) {
        if (inlineImg || !ref || !ref->isRef()) {
            return false;
        }
        const Ref candidate = ref->getRef();
        return candidate.num == targetRef.num && candidate.gen == targetRef.gen;
    }

    if (!matchByOccurrence) {
        return false;
    }

    if (imageType != requestedType) {
        return false;
    }

    if (seenOccurrences == targetOccurrence) {
        ++seenOccurrences;
        return true;
    }

    ++seenOccurrences;
    return false;
}

void ImageOutputDev::writeRawImage(Stream *str,
                                   ImageExtension ext,
                                   ImageType type,
                                   int width,
                                   int height,
                                   int components,
                                   int bitsPerComponent,
                                   double widthDPI,
                                   double heightDPI,
                                   const std::vector<uint8_t> *jbig2Globals,
                                   const CcittParams *ccittParams)
{
    if (!outputBuffer) {
        return;
    }

    Stream *dataStream = str->getNextStream();
    if (!dataStream->reset()) {
        error(errIO, -1, "Couldn't reset image stream");
        errorCode = 2;
        return;
    }

    std::vector<uint8_t> buffer;
    buffer.reserve(64 * 1024);
    int c;
    while ((c = dataStream->getChar()) != EOF) {
        buffer.push_back(static_cast<uint8_t>(c));
    }

    dataStream->close();
    const uint32_t w = width > 0 ? static_cast<uint32_t>(width) : 0;
    const uint32_t h = height > 0 ? static_cast<uint32_t>(height) : 0;
    const uint32_t comps = components > 0 ? static_cast<uint32_t>(components) : 0;
    const uint32_t bpc = bitsPerComponent > 0 ? static_cast<uint32_t>(bitsPerComponent) : 0;
    storeResult(buffer, imgUnknown, ext, type, w, h, 0, comps, bpc, widthDPI, heightDPI, jbig2Globals, ccittParams);
}

void ImageOutputDev::writeImageFile(Stream *str,
                                    ImageFormat format,
                                    ImageExtension ext,
                                    ImageType type,
                                    int width,
                                    int height,
                                    GfxImageColorMap *colorMap,
                                    double widthDPI,
                                    double heightDPI)
{
    if (!outputBuffer) {
        return;
    }

    ImageStream *imgStr = nullptr;
    if (format != imgMonochrome) {
        if (!colorMap) {
            error(errInternal, -1, "Missing color map for raster extraction");
            errorCode = 99;
            return;
        }
        imgStr = new ImageStream(str, width, colorMap->getNumPixelComps(), colorMap->getBits());
        if (!imgStr->reset()) {
            error(errIO, -1, "Stream reset failed");
            errorCode = 3;
            delete imgStr;
            return;
        }
    } else {
        if (!str->reset()) {
            error(errIO, -1, "Stream reset failed");
            errorCode = 3;
            return;
        }
    }

    uint32_t stride = 0;
    uint32_t components = 0;
    uint32_t bitsPerComponent = 8;
    switch (format) {
    case imgRGB:
        components = 3;
        stride = width * components;
        break;
    case imgRGB48:
        components = 3;
        bitsPerComponent = 16;
        stride = width * components * 2;
        break;
    case imgCMYK:
        components = 4;
        stride = width * components;
        break;
    case imgGray:
        components = 1;
        stride = width;
        break;
    case imgMonochrome:
        components = 1;
        bitsPerComponent = 1;
        stride = (width + 7) / 8;
        break;
    case imgUnknown:
        components = 0;
        stride = 0;
        break;
    }

    std::vector<uint8_t> raster(static_cast<size_t>(stride) * height);

    GfxRGB rgb;
    GfxCMYK cmyk;
    GfxGray gray;
    unsigned char *row = nullptr;
    unsigned char zero[gfxColorMaxComps];
    int invert_bits = 0xff;

    if (format == imgMonochrome) {
        if (colorMap) {
            memset(zero, 0, sizeof(zero));
            colorMap->getGray(zero, &gray);
            if (colToByte(gray) == 0) {
                invert_bits = 0x00;
            }
        }
    } else {
        const size_t rowSize = stride;
        row = static_cast<unsigned char *>(gmallocn_checkoverflow(rowSize, 1));
        if (!row) {
            error(errIO, -1, "Unable to allocate temporary row buffer");
            if (imgStr) {
                imgStr->close();
                delete imgStr;
            }
            errorCode = 99;
            return;
        }
    }

    for (int y = 0; y < height; ++y) {
        unsigned char *dest = raster.data() + static_cast<size_t>(stride) * y;
        switch (format) {
        case imgRGB: {
            unsigned char *rowp = row;
            unsigned char *p = imgStr->getLine();
            for (int x = 0; x < width; ++x) {
                if (p) {
                    colorMap->getRGB(p, &rgb);
                    *rowp++ = colToByte(rgb.r);
                    *rowp++ = colToByte(rgb.g);
                    *rowp++ = colToByte(rgb.b);
                    p += colorMap->getNumPixelComps();
                } else {
                    *rowp++ = 0;
                    *rowp++ = 0;
                    *rowp++ = 0;
                }
            }
            memcpy(dest, row, stride);
            break;
        }
        case imgRGB48: {
            auto *row16 = reinterpret_cast<unsigned short *>(row);
            unsigned char *p = imgStr->getLine();
            for (int x = 0; x < width; ++x) {
                if (p) {
                    colorMap->getRGB(p, &rgb);
                    *row16++ = colToShort(rgb.r);
                    *row16++ = colToShort(rgb.g);
                    *row16++ = colToShort(rgb.b);
                    p += colorMap->getNumPixelComps();
                } else {
                    *row16++ = 0;
                    *row16++ = 0;
                    *row16++ = 0;
                }
            }
            memcpy(dest, row, stride);
            break;
        }
        case imgCMYK: {
            unsigned char *rowp = row;
            unsigned char *p = imgStr->getLine();
            for (int x = 0; x < width; ++x) {
                if (p) {
                    colorMap->getCMYK(p, &cmyk);
                    *rowp++ = colToByte(cmyk.c);
                    *rowp++ = colToByte(cmyk.m);
                    *rowp++ = colToByte(cmyk.y);
                    *rowp++ = colToByte(cmyk.k);
                    p += colorMap->getNumPixelComps();
                } else {
                    *rowp++ = 0;
                    *rowp++ = 0;
                    *rowp++ = 0;
                    *rowp++ = 0;
                }
            }
            memcpy(dest, row, stride);
            break;
        }
        case imgGray: {
            unsigned char *rowp = row;
            unsigned char *p = imgStr->getLine();
            for (int x = 0; x < width; ++x) {
                if (p) {
                    colorMap->getGray(p, &gray);
                    *rowp++ = colToByte(gray);
                    p += colorMap->getNumPixelComps();
                } else {
                    *rowp++ = 0;
                }
            }
            memcpy(dest, row, stride);
            break;
        }
        case imgMonochrome: {
            const int size = (width + 7) / 8;
            for (int x = 0; x < size; ++x) {
                dest[x] = static_cast<unsigned char>(str->getChar() ^ invert_bits);
            }
            break;
        }
        case imgUnknown:
            break;
        }
    }

    if (row) {
        gfree(row);
    }

    if (format != imgMonochrome) {
        imgStr->close();
        delete imgStr;
    }

    str->close();

    storeResult(raster,
                format,
                ext,
                type,
                static_cast<uint32_t>(width),
                static_cast<uint32_t>(height),
                stride,
                components,
                bitsPerComponent,
                widthDPI,
                heightDPI,
                nullptr,
                nullptr);
}

void ImageOutputDev::writeImage(GfxState *state,
                                Object *ref,
                                Stream *str,
                                int width,
                                int height,
                                GfxImageColorMap *colorMap,
                                bool inlineImg,
                                ImageType imageType)
{
    if (!matchesTarget(ref, inlineImg, imageType)) {
        return;
    }

    const double widthDPI = kDefaultImageDPI;
    const double heightDPI = kDefaultImageDPI;

    EmbedStream *embedStr = nullptr;
    if (inlineImg) {
        embedStr = static_cast<EmbedStream *>(str->getBaseStream());
        getInlineImageLength(str, width, height, colorMap);
        embedStr->rewind();
    }

    const int components = colorMap ? colorMap->getNumPixelComps() : (imageType == imgMask || imageType == imgStencil ? 1 : 0);
    const int bitsPerComponent = colorMap ? colorMap->getBits() : (imageType == imgMask || imageType == imgStencil ? 1 : 0);

    const StreamKind kind = str->getKind();
    if (kind == strDCT) {
        writeRawImage(str, extJpg, imageType, width, height, components, bitsPerComponent, widthDPI, heightDPI, nullptr, nullptr);
        if (inlineImg && embedStr) {
            embedStr->restore();
        }
        return;
    }
    if (kind == strJPX && !inlineImg) {
        writeRawImage(str, extJp2, imageType, width, height, components, bitsPerComponent, widthDPI, heightDPI, nullptr, nullptr);
        if (inlineImg && embedStr) {
            embedStr->restore();
        }
        return;
    }
    if (kind == strJBIG2 && !inlineImg) {
        std::vector<uint8_t> globals;
        if (auto *jbig2 = dynamic_cast<JBIG2Stream *>(str); jbig2) {
            if (Object *globalsObj = jbig2->getGlobalsStream(); globalsObj && globalsObj->isStream()) {
                Stream *globalsStream = globalsObj->getStream();
                if (globalsStream && globalsStream->reset()) {
                    int c;
                    while ((c = globalsStream->getChar()) != EOF) {
                        globals.push_back(static_cast<uint8_t>(c));
                    }
                    globalsStream->close();
                }
                globalsObj->streamClose();
            }
        }
        writeRawImage(str, extJb2e, imageType, width, height, components, bitsPerComponent, widthDPI, heightDPI, &globals, nullptr);
        if (inlineImg && embedStr) {
            embedStr->restore();
        }
        return;
    }
    if (kind == strCCITTFax) {
        CcittParams params {};
        const CcittParams *paramsPtr = nullptr;
        if (auto *ccitt = dynamic_cast<CCITTFaxStream *>(str); ccitt) {
            params.encoding = ccitt->getEncoding();
            params.endOfLine = ccitt->getEndOfLine();
            params.byteAlign = ccitt->getEncodedByteAlign();
            params.columns = ccitt->getColumns();
            params.rows = height;
            params.endOfBlock = ccitt->getEndOfBlock();
            params.blackIs1 = ccitt->getBlackIs1();
            params.damagedRowsBeforeError = ccitt->getDamagedRowsBeforeError();
            paramsPtr = &params;
        }
        writeRawImage(str, extCcitt, imageType, width, height, 1, 1, widthDPI, heightDPI, nullptr, paramsPtr);
        if (inlineImg && embedStr) {
            embedStr->restore();
        }
        return;
    }

    const ImageFormat format = determineFormat(colorMap);
    const ImageExtension ext = defaultExtension(format);
    writeImageFile(str, format, ext, imageType, width, height, colorMap, widthDPI, heightDPI);

    if (inlineImg && embedStr) {
        embedStr->restore();
    }
}

bool ImageOutputDev::tilingPatternFill(GfxState *state,
                                       Gfx *gfx,
                                       Catalog *cat,
                                       GfxTilingPattern *tPat,
                                       const std::array<double, 6> &mat,
                                       int x0,
                                       int y0,
                                       int x1,
                                       int y1,
                                       double xStep,
                                       double yStep)
{
    (void)state;
    (void)gfx;
    (void)cat;
    (void)tPat;
    (void)mat;
    (void)x0;
    (void)y0;
    (void)x1;
    (void)y1;
    (void)xStep;
    (void)yStep;
    return true;
}

void ImageOutputDev::drawImageMask(GfxState *state,
                                   Object *ref,
                                   Stream *str,
                                   int width,
                                   int height,
                                   bool invert [[maybe_unused]],
                                   bool interpolate [[maybe_unused]],
                                   bool inlineImg)
{
    writeImage(state, ref, str, width, height, nullptr, inlineImg, imgStencil);
}

void ImageOutputDev::drawImage(GfxState *state,
                               Object *ref,
                               Stream *str,
                               int width,
                               int height,
                               GfxImageColorMap *colorMap,
                               bool interpolate [[maybe_unused]],
                               const int *maskColors [[maybe_unused]],
                               bool inlineImg)
{
    writeImage(state, ref, str, width, height, colorMap, inlineImg, imgImage);
}

void ImageOutputDev::drawMaskedImage(GfxState *state,
                                     Object *ref,
                                     Stream *str,
                                     int width,
                                     int height,
                                     GfxImageColorMap *colorMap,
                                     bool interpolate [[maybe_unused]],
                                     Stream *maskStr,
                                     int maskWidth,
                                     int maskHeight,
                                     bool maskInvert [[maybe_unused]],
                                     bool maskInterpolate [[maybe_unused]])
{
    writeImage(state, ref, str, width, height, colorMap, false, imgImage);
    writeImage(state, ref, maskStr, maskWidth, maskHeight, nullptr, false, imgMask);
}

void ImageOutputDev::drawSoftMaskedImage(GfxState *state,
                                         Object *ref,
                                         Stream *str,
                                         int width,
                                         int height,
                                         GfxImageColorMap *colorMap,
                                         bool interpolate [[maybe_unused]],
                                         Stream *maskStr,
                                         int maskWidth,
                                         int maskHeight,
                                         GfxImageColorMap *maskColorMap,
                                         bool maskInterpolate [[maybe_unused]])
{
    writeImage(state, ref, str, width, height, colorMap, false, imgImage);
    writeImage(state, ref, maskStr, maskWidth, maskHeight, maskColorMap, false, imgSmask);
}

