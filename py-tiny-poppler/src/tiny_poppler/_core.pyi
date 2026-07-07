"""tiny-poppler core extension"""

import enum

import numpy

class ColorMode(enum.Enum):
    Mono1 = 0

    Mono8 = 1

    Rgb8 = 2

    Bgr8 = 3

    Xbgr8 = 4

    Cmyk8 = 5

    DeviceN8 = 6

class ImageColorSpace(enum.Enum):
    Unknown = 0

    DeviceGray = 1

    DeviceRgb = 2

    DeviceCmyk = 3

    Lab = 4

    Icc = 5

    Indexed = 6

    Pattern = 7

    Separation = 8

    DeviceN = 9

class ImageType(enum.Enum):
    Unknown = 0

    Image = 1

    Stencil = 2

    Mask = 3

    SoftMask = 4

class CropMode(enum.Enum):
    Unknown = 0

    MediaBox = 1

    CropBox = 2

class ZeroWidthLineMode(enum.Enum):
    Default = 0

    Hairline = 1

    Nothing = 2

class ImageExportMatchMode(enum.Enum):
    ByRef = 0

    ByOccurrence = 1

class ImageExportType(enum.Enum):
    Image = 0

    Stencil = 1

    Mask = 2

    SoftMask = 3

class ImageExportFormat(enum.Enum):
    Unknown = 0

    Rgb = 1

    Rgb48 = 2

    Gray = 3

    Monochrome = 4

    Cmyk = 5

class ImageExportExtension(enum.Enum):
    Jpeg = 0

    Jp2 = 1

    Jbig2 = 2

    Ccitt = 3

    Png = 4

    Tiff = 5

    Pnm = 6

class XYZColor:
    @property
    def x(self) -> float: ...
    @property
    def y(self) -> float: ...
    @property
    def z(self) -> float: ...
    def __repr__(self) -> str: ...

class MinMaxRange:
    @property
    def min(self) -> float: ...
    @property
    def max(self) -> float: ...
    def __repr__(self) -> str: ...

class ColorSpace:
    @property
    def mode(self) -> ImageColorSpace: ...
    @property
    def separation_name(self) -> str | None: ...
    @property
    def indexed_hival(self) -> int: ...
    @property
    def devicen_names(self) -> list[str]: ...
    @property
    def lab_white(self) -> XYZColor | None: ...
    @property
    def lab_black(self) -> XYZColor | None: ...
    @property
    def lab_a(self) -> MinMaxRange | None: ...
    @property
    def lab_b(self) -> MinMaxRange | None: ...
    @property
    def alternate(self) -> object | None: ...
    def __repr__(self) -> str: ...

class ImageInfo:
    @property
    def width(self) -> int: ...
    @property
    def height(self) -> int: ...
    @property
    def components(self) -> int: ...
    @property
    def bits_per_component(self) -> int: ...
    @property
    def page(self) -> int: ...
    @property
    def image_type(self) -> ImageType: ...
    @property
    def colorspace(self) -> ImageColorSpace: ...
    @property
    def color_space(self) -> object: ...
    def xref(self) -> tuple[int, int] | None: ...
    def dpi(self) -> tuple[float, float]: ...
    def matrix(self) -> tuple[float, float, float, float, float, float]: ...
    def __repr__(self) -> str: ...

class PageInfo:
    @property
    def page_number(self) -> int: ...
    @property
    def image_count(self) -> int: ...
    @property
    def object_count(self) -> int: ...
    @property
    def is_pdf_a_compatible(self) -> bool: ...
    @property
    def colorspaces(self) -> dict: ...
    def crop_box(self) -> tuple[float, float, float, float] | None: ...
    def media_box(self) -> tuple[float, float, float, float] | None: ...
    def __repr__(self) -> str: ...

class ImageCollection:
    @property
    def images(self) -> list[ImageInfo]: ...
    @property
    def pages(self) -> list[PageInfo]: ...
    def __repr__(self) -> str: ...

class RenderedImage:
    @property
    def data(self) -> numpy.ndarray: ...
    @property
    def width(self) -> int: ...
    @property
    def height(self) -> int: ...
    @property
    def stride(self) -> int: ...
    @property
    def components(self) -> int: ...
    @property
    def bits_per_component(self) -> int: ...
    @property
    def color_mode(self) -> ColorMode: ...
    def __repr__(self) -> str: ...

class CcittParams:
    @property
    def encoding(self) -> int: ...
    @property
    def columns(self) -> int: ...
    @property
    def rows(self) -> int: ...
    @property
    def damaged_rows_before_error(self) -> int: ...
    @property
    def end_of_line(self) -> bool: ...
    @property
    def byte_align(self) -> bool: ...
    @property
    def end_of_block(self) -> bool: ...
    @property
    def black_is_one(self) -> bool: ...
    def __repr__(self) -> str: ...

class ExportedImage:
    @property
    def data(self) -> bytes: ...
    @property
    def jbig2_globals(self) -> bytes | None: ...
    @property
    def ccitt_params(self) -> CcittParams | None: ...
    @property
    def width(self) -> int: ...
    @property
    def height(self) -> int: ...
    @property
    def stride(self) -> int: ...
    @property
    def components(self) -> int: ...
    @property
    def bits_per_component(self) -> int: ...
    @property
    def width_dpi(self) -> float: ...
    @property
    def height_dpi(self) -> float: ...
    @property
    def format(self) -> ImageExportFormat: ...
    @property
    def type(self) -> ImageExportType: ...
    @property
    def extension(self) -> ImageExportExtension: ...
    @property
    def has_jbig2_globals(self) -> bool: ...
    def __repr__(self) -> str: ...

class Document:
    @staticmethod
    def open(path: str, owner_password: str | None = None, user_password: str | None = None) -> Document: ...
    @property
    def page_count(self) -> int: ...
    def render_page(
        self,
        page_index: int,
        dpi: float = 150.0,
        color_mode: ColorMode = ColorMode.Rgb8,
        crop_mode: CropMode = CropMode.CropBox,
        zero_width_line_mode: ZeroWidthLineMode = ZeroWidthLineMode.Default,
    ) -> RenderedImage: ...
    def collect_images(self, page_start: int = 1, page_end: int = 0) -> ImageCollection: ...
    def export_image(
        self,
        page_index: int,
        xref_object: int | None = None,
        xref_generation: int = 0,
        occurrence_index: int = 0,
        target_type: ImageExportType = ImageExportType.Image,
        describe_only: bool = False,
    ) -> ExportedImage: ...
    def __repr__(self) -> str: ...

def get_version() -> tuple[int, int, int]: ...
