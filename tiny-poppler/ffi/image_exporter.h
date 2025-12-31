//========================================================================
//
// ImageOutputDev.h
//
// Copyright 1998-2003 Glyph & Cog, LLC
//
//========================================================================

//========================================================================
//
// Modified under the Poppler project - http://poppler.freedesktop.org
// And modified again for tiny-poppler/npdf project + renamed into
// image_exporter.h - https://github.com/noaione/npdf
//
// All changes made under the Poppler project to this file are licensed
// under GPL version 2 or later
//
// Meanwhile, all changes made under the tiny-poppler/npdf are licensed
// under GPL version 3 or later
//
// Copyright (C) 2006 Rainer Keller <class321@gmx.de>
// Copyright (C) 2008 Timothy Lee <timothy.lee@siriushk.com>
// Copyright (C) 2009 Carlos Garcia Campos <carlosgc@gnome.org>
// Copyright (C) 2010 Jakob Voss <jakob.voss@gbv.de>
// Copyright (C) 2012, 2013, 2017 Adrian Johnson <ajohnson@redneon.com>
// Copyright (C) 2013 Thomas Freitag <Thomas.Freitag@alfa.de>
// Copyright (C) 2018, 2019, 2021, 2024, 2025 Albert Astals Cid <aacid@kde.org>
// Copyright (C) 2024 Fernando Herrera <fherrera@onirica.com>
// Copyright (C) 2024 Sebastian J. Bronner <waschtl@sbronner.com>
// Copyright (C) 2025 noaione <noaione@n4o.xyz>
//
//========================================================================

#ifndef TINY_POPPLER_IMAGEOUTPUTDEV_H
#define TINY_POPPLER_IMAGEOUTPUTDEV_H

#include "poppler/poppler-config.h"

#include "Object.h"
#include "OutputDev.h"
#include <array>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <vector>

class GfxState;

//------------------------------------------------------------------------
// ImageOutputDev
//------------------------------------------------------------------------

class NTImageOutputDev : public OutputDev
{
  public:
    enum NTImageType
    {
        ntImgImage,
        ntImgStencil,
        ntImgMask,
        ntImgSmask
    };
    enum NTImageFormat
    {
        ntImgUnknown,
        ntImgRGB,
        ntImgRGB48,
        ntImgGray,
        ntImgMonochrome,
        ntImgCMYK
    };
    enum NTImageExtension
    {
        ntExtJpg,   // JPEG
        ntExtJp2,   // JPEG 2000
        ntExtJb2e,  // JBIG2 embedded
        ntExtCcitt, // CCITT Group 4
        ntExtPng,   // PNG
        ntExtTiff,  // TIFF
        ntExtPnm,   // PNM (PBM/PGM/PPM) - Use ppm if RGB, else pbm
    };

    struct NTCcittParams {
        int encoding;
        bool endOfLine;
        bool byteAlign;
        int columns;
        int rows;
        bool endOfBlock;
        bool blackIs1;
        int damagedRowsBeforeError;
    };

    struct NTImageOutput {
        uint8_t *data;
        size_t len;
        uint32_t width;
        uint32_t height;
        uint32_t stride;
        uint32_t components;
        uint32_t bits_per_component;
        double wDPI;
        double hDPI;
        NTImageFormat format;
        NTImageType type;
        NTImageExtension extension;
        bool hasJbig2Globals;
        uint8_t *jbig2Globals;
        size_t jbig2GlobalsLen;
        bool hasCcittParams;
        NTCcittParams ccittParams;
    };

    // Create an extractor targeting a specific image reference. If matchRef is false,
    // the first image of the requested type will be captured.
    NTImageOutputDev(NTImageOutput *outputBuffer, const Ref &targetRef, bool matchRef,
                     NTImageType targetType, bool matchOccurrence, uint32_t occurrenceIndex);

    // Destructor.
    ~NTImageOutputDev() override;

    // Does this device use tilingPatternFill()?  If this returns false,
    // tiling pattern fills will be reduced to a series of other drawing
    // operations.
    bool useTilingPatternFill() override { return true; }

    // Does this device use beginType3Char/endType3Char?  Otherwise,
    // text in Type 3 fonts will be drawn with drawChar/drawString.
    bool interpretType3Chars() override { return false; }

    // Does this device need non-text content?
    bool needNonText() override { return true; }

    // Set whether to only describe images instead of extracting them
    void setDescribeOnly(bool describeOnlyA) { describeOnly = describeOnlyA; }

    // Get the error code
    // 0 = No error, 1 = Error opening a PDF file, 2 = Error opening an output file, 3 = Error
    // related to PDF permissions, 99 = Other error.
    int getErrorCode() const { return errorCode; }
    bool hasCaptured() const { return captured; }

    // Start a page
    void startPage(int pageNumA, GfxState *state, XRef *xref) override
    {
        (void)pageNumA;
        (void)state;
        (void)xref;
    }

    //---- get info about output device

    // Does this device use upside-down coordinates?
    // (Upside-down means (0,0) is the top left corner of the page.)
    bool upsideDown() override { return true; }

    // Does this device use drawChar() or drawString()?
    bool useDrawChar() override { return false; }

    //----- path painting
    bool tilingPatternFill(GfxState *state, Gfx *gfx, Catalog *cat, GfxTilingPattern *tPat,
                           const std::array<double, 6> &mat, int x0, int y0, int x1, int y1,
                           double xStep, double yStep) override;

    //----- image drawing
    void drawImageMask(GfxState *state, Object *ref, Stream *str, int width, int height,
                       bool invert, bool interpolate, bool inlineImg) override;
    void drawImage(GfxState *state, Object *ref, Stream *str, int width, int height,
                   GfxImageColorMap *colorMap, bool interpolate, const int *maskColors,
                   bool inlineImg) override;
    void drawMaskedImage(GfxState *state, Object *ref, Stream *str, int width, int height,
                         GfxImageColorMap *colorMap, bool interpolate, Stream *maskStr,
                         int maskWidth, int maskHeight, bool maskInvert,
                         bool maskInterpolate) override;
    void drawSoftMaskedImage(GfxState *state, Object *ref, Stream *str, int width, int height,
                             GfxImageColorMap *colorMap, bool interpolate, Stream *maskStr,
                             int maskWidth, int maskHeight, GfxImageColorMap *maskColorMap,
                             bool maskInterpolate) override;

  private:
    void writeImage(GfxState *state, Object *ref, Stream *str, int width, int height,
                    GfxImageColorMap *colorMap, bool inlineImg, NTImageType imageType);
    void writeRawImage(Stream *str, NTImageExtension ext, NTImageType type, int width, int height,
                       int components, int bitsPerComponent, double widthDPI, double heightDPI,
                       const std::vector<uint8_t> *jbig2Globals, const NTCcittParams *ccittParams);
    void writeImageFile(Stream *str, NTImageFormat format, NTImageExtension ext, NTImageType type,
                        int width, int height, GfxImageColorMap *colorMap, double widthDPI,
                        double heightDPI);
    long getInlineImageLength(Stream *str, int width, int height, GfxImageColorMap *colorMap);
    bool matchesTarget(Object *ref, bool inlineImg, NTImageType imageType) const;
    void storeResult(const std::vector<uint8_t> &buffer, NTImageFormat format, NTImageExtension ext,
                     NTImageType type, uint32_t width, uint32_t height, uint32_t stride,
                     uint32_t components, uint32_t bitsPerComponent, double widthDPI,
                     double heightDPI, const std::vector<uint8_t> *jbig2Globals,
                     const NTCcittParams *ccittParams);

    Ref targetRef;                  // reference to match
    bool matchByRef = true;         // match by object reference
    bool matchByOccurrence = false; // capture nth occurrence of requested type
    NTImageType requestedType;      // requested image type when matching by occurrence
    uint32_t targetOccurrence =
        0; // specific occurrence index to capture when matching by occurrence
    mutable uint32_t seenOccurrences = 0;  // number of matched images observed so far
    bool captured = false;                 // true once an image has been extracted
    NTImageOutput *outputBuffer = nullptr; // output buffer for images
    int errorCode;                         // code for any error creating the output files
    bool describeOnly = false;
};

#endif
