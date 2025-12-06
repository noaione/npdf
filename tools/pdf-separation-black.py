import sys
from pathlib import Path

import pikepdf
from pikepdf import Array, ContentStreamInstruction, Dictionary, Name, Operator

try:
    file_path = sys.argv[1]
except IndexError:
    print("Usage: python3 pdf-separation-black.py <pdf-file>")
    sys.exit(1)

target_file = Path(file_path)
if not target_file.is_file():
    print(f"Error: File not found: {target_file}")
    sys.exit(1)

pdf = pikepdf.Pdf.open(target_file)

tint_transform = Dictionary({
    "/FunctionType": 2,
    "/Domain": [0, 1],
    "/Range": [0, 1],
    "/C0": [1.0],  # Input 0 -> White
    "/C1": [0.0],  # Input 1 -> Black
    "/N": 1.0,  # Linear interpolation
})

sep_cs_array = Array([Name("/Separation"), Name("/All"), Name("/DeviceGray"), tint_transform])


def in_place_page_with_separation_black(page: pikepdf.Page, sep_cs: pikepdf.Array) -> None:
    if "/ColorSpace" not in page.Resources:
        page.Resources["/ColorSpace"] = Dictionary()

    page.Resources["/ColorSpace"]["/PureBlackCS"] = sep_cs  # Add the new CS

    instructions = pikepdf.parse_content_stream(page)
    new_instructions = []
    is_before_cs = False
    # has_weird_operand = False
    for instruction in instructions:
        # print(instruction)
        if instruction.operator == Operator("cs") or instruction.operator == Operator("CS"):
            # Replace the operand with our new color space
            new_instruct = ContentStreamInstruction([Name("/PureBlackCS")], instruction.operator)
            new_instructions.append(new_instruct)
            is_before_cs = True
        elif instruction.operator == Operator("scn") or instruction.operator == Operator("SCN"):
            # 'scn' -> fill, 'SCN' -> stroke
            if not is_before_cs:
                new_instructions.append(instruction)  # Keep as-is
                # has_weird_operand = True
                continue

            new_instructions.append(
                ContentStreamInstruction(
                    [1.0],  # Black in the Separation color space
                    instruction.operator,
                )
            )
            is_before_cs = False
        else:
            new_instructions.append(instruction)

    # if has_weird_operand:
    #     print("Warning: Found 'scn' or 'SCN' operator before color space setting; content stream may be complex.")
    #     for instr in new_instructions:
    #         print(instr)
    #     raise RuntimeError("Aborting due to complex content stream.")
    new_content_stream = pikepdf.unparse_content_stream(new_instructions)
    page.Contents = pdf.make_stream(new_content_stream)


def is_pure_stencil_page(page: pikepdf.Page) -> bool:
    """
    Returns True ONLY if:
    1. The page draws at least one Stencil Image.
    2. The page draws NO normal images, forms, or images with SMasks/Masks.

    Created with Gemini 3, don't know if it's perfect but it's working well so far.
    """

    safe_stencils = set()
    forbidden_objects = set()

    if "/XObject" not in page.Resources:
        return False  # No images = No replacement needed

    for name, xobj in page.Resources.XObject.items():
        subtype = xobj.get("/Subtype")

        # If it's not an Image (e.g., /Form), it's automatically forbidden
        # (Forms are complex containers that might hide normal images inside)
        if subtype != "/Image":
            forbidden_objects.add(name)
            continue

        # Check image properties
        is_image_mask = xobj.get("/ImageMask")
        has_smask = "/SMask" in xobj
        has_mask = "/Mask" in xobj

        # Must be an ImageMask, and must NOT have complex transparency attached
        if is_image_mask and not has_smask and not has_mask:
            safe_stencils.add(name)
        else:
            # It's a normal photo, or a stencil with complex alpha blending
            forbidden_objects.add(name)

    try:
        commands = pikepdf.parse_content_stream(page)
    except (TypeError, pikepdf.PdfError):
        return False  # Corrupt stream

    has_drawn_stencil = False

    for operands, operator in commands:
        op = str(operator)

        # Check for XObject drawing operator 'Do'
        if op == "Do":
            obj_name = operands[0]

            # found some forbidden object, ignore
            if obj_name in forbidden_objects:
                return False

            # found stencil, mark as found
            if obj_name in safe_stencils:
                has_drawn_stencil = True

        # Inline image, most likely not stencil
        elif op == "BI":
            return False

    # Only return True if we actually drew a stencil and didn't hit any forbidden objects
    return has_drawn_stencil


for pg_idx, page in enumerate(pdf.pages, start=1):
    if is_pure_stencil_page(page):
        print(f"Page {pg_idx} is pure stencil - applying separation black replacement")
        in_place_page_with_separation_black(page, sep_cs_array)

new_pdf_path = target_file.with_stem(target_file.stem + "-sep-black")
pdf.save(new_pdf_path)
