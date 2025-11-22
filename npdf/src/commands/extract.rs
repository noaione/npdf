use clap::Args;
use color_print::{cformat, cprintln};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use tiny_poppler::{
    Document, EncodedExportedImage, ExportedImage, ImageExportRequest, ImageExportSelector,
    ImageExportType, ImageInfo, ImageSinkOptions, ImageType, PdfPasswords, sink_exported_image,
};

#[derive(Args)]
pub struct ExtractArgs {
    /// Path to the PDF file containing embedded images.
    pub pdf: PathBuf,
    /// Directory where extracted images will be written.
    pub output: PathBuf,
    /// First 1-based page to inspect (defaults to the first page).
    #[arg(long, value_name = "PAGE")]
    pub first: Option<u32>,
    /// Last 1-based page to inspect (defaults to the final page).
    #[arg(long, value_name = "PAGE")]
    pub last: Option<u32>,
    /// Describe discovered images without writing any files.
    #[arg(long)]
    pub describe: bool,
}

pub fn run(args: ExtractArgs, passwords: Option<&PdfPasswords>) -> Result<(), String> {
    if !args.pdf.exists() {
        return Err(format!("PDF file does not exist: {}", args.pdf.display()));
    }

    let mut document =
        Document::open_with_passwords(&args.pdf, passwords).map_err(|err| err.to_string())?;
    let page_count = document.page_count().map_err(|err| err.to_string())?;

    let first_page = args.first.unwrap_or(1);
    let last_page = args.last.unwrap_or(page_count);

    if first_page == 0 || first_page > page_count {
        return Err(format!(
            "first page must be between 1 and {page_count} (got {first_page})"
        ));
    }
    if last_page == 0 || last_page > page_count {
        return Err(format!(
            "last page must be between 1 and {page_count} (got {last_page})"
        ));
    }
    if last_page < first_page {
        return Err("last page must be greater than or equal to first page".into());
    }

    let images = document
        .images_in_range(first_page, last_page)
        .map_err(|err| format!("failed to enumerate images: {err}"))?;

    if images.is_empty() {
        cprintln!(
            "<yellow>No embedded images found between page <c,s>{}</c,s> and <c,s>{}</c,s>.</yellow>",
            first_page,
            last_page
        );
        return Ok(());
    }

    let mut grouped: BTreeMap<u32, Vec<ImageInfo>> = BTreeMap::new();
    for info in images {
        grouped.entry(info.page).or_default().push(info);
    }

    if !args.describe {
        fs::create_dir_all(&args.output)
            .map_err(|err| format!("failed to create output directory: {err}"))?;
    }

    let mut total_groups = 0usize;
    let mut total_components = 0usize;

    for page in first_page..=last_page {
        let page_infos = grouped.get(&page).cloned().unwrap_or_default();
        if page_infos.is_empty() {
            continue;
        }

        let groups = build_page_groups(page, page_infos);
        if groups.is_empty() {
            continue;
        }

        let use_slots = groups.len() > 1;
        for (group_idx, group) in groups.into_iter().enumerate() {
            total_groups += 1;
            let slot_suffix = if use_slots {
                format!("-{num:02}", num = group_idx + 1)
            } else {
                String::new()
            };
            let slot_display = if use_slots { Some(group_idx + 1) } else { None };

            for (component_suffix, entry) in group.into_components() {
                let exported = export_image_entry(&mut document, &entry)
                    .map_err(|err| format!("{} (page {})", err, entry.info.page))?;

                if args.describe {
                    describe_component(
                        page,
                        slot_display,
                        component_suffix,
                        &entry.info,
                        &exported,
                    );
                } else {
                    let encoded = sink_exported_image(exported, ImageSinkOptions::default())
                        .map_err(|err| format!("failed to encode image: {err}"))?;
                    let extension = encoded.file_extension();
                    let path = build_output_path(
                        &args.output,
                        page,
                        &slot_suffix,
                        component_suffix,
                        extension,
                    );
                    persist_encoded(
                        &path,
                        page,
                        slot_display,
                        component_suffix,
                        &entry.info,
                        &encoded,
                    )?;
                }

                total_components += 1;
            }
        }
    }

    if total_groups == 0 {
        cprintln!(
            "<yellow>No embedded images found between page <c,s>{}</c,s> and <c,s>{}</c,s>.</yellow>",
            first_page,
            last_page
        );
        return Ok(());
    }

    if !args.describe {
        cprintln!(
            "Saved <m,s>{}</m,s> image component(s) across <m,s>{}</m,s> group(s) between page <c,s>{}</c,s> and <c,s>{}</c,s>.",
            total_components,
            total_groups,
            first_page,
            last_page
        );
    }

    Ok(())
}

#[derive(Clone)]
struct ImageEntry {
    info: ImageInfo,
    selector: ExtractionSelector,
    target_type: ImageExportType,
}

impl ImageEntry {
    fn new(info: ImageInfo, selector: ExtractionSelector, target_type: ImageExportType) -> Self {
        Self {
            info,
            selector,
            target_type,
        }
    }
}

#[derive(Clone)]
enum ExtractionSelector {
    Reference { object: i32, generation: i32 },
    Inline { occurrence: u32 },
}

impl ExtractionSelector {
    fn to_request(&self) -> ImageExportSelector {
        match self {
            ExtractionSelector::Reference { object, generation } => {
                ImageExportSelector::Reference {
                    object: *object,
                    generation: *generation,
                }
            }
            ExtractionSelector::Inline { occurrence } => ImageExportSelector::NthOfType {
                occurrence: *occurrence,
            },
        }
    }
}

#[derive(Clone)]
struct ImageGroup {
    primary: ImageEntry,
    mask: Option<ImageEntry>,
    soft_mask: Option<ImageEntry>,
}

impl ImageGroup {
    fn new(primary: ImageEntry) -> Self {
        Self {
            primary,
            mask: None,
            soft_mask: None,
        }
    }

    fn into_components(self) -> Vec<(&'static str, ImageEntry)> {
        let mut entries = Vec::with_capacity(3);
        entries.push(("", self.primary));
        if let Some(mask) = self.mask {
            entries.push(("-mask", mask));
        }
        if let Some(smask) = self.soft_mask {
            entries.push(("-smask", smask));
        }
        entries
    }
}

#[derive(Clone, Copy)]
enum ComponentKind {
    Mask,
    SoftMask,
}

fn build_page_groups(page: u32, entries: Vec<ImageInfo>) -> Vec<ImageGroup> {
    let mut groups = Vec::new();
    let mut inline_counters: HashMap<u32, u32> = HashMap::new();
    let mut xref_index: HashMap<(i32, i32), usize> = HashMap::new();

    for info in entries.into_iter() {
        let target_type = match export_type_from_image(info.image_type) {
            Some(kind) => kind,
            None => {
                cprintln!(
                    "<yellow>Skipping unsupported image type {:?} on page <c,s>{}</c,s>.</yellow>",
                    info.image_type,
                    page
                );
                continue;
            }
        };

        let selector = if let Some((object, generation)) = info.xref {
            ExtractionSelector::Reference { object, generation }
        } else {
            let key = target_type as u32;
            let counter = inline_counters.entry(key).or_insert(0);
            let selector = ExtractionSelector::Inline {
                occurrence: *counter,
            };
            *counter += 1;
            selector
        };

        match info.image_type {
            ImageType::Image | ImageType::Stencil => {
                let maybe_xref = info.xref;
                let entry = ImageEntry::new(info, selector, target_type);
                let idx = groups.len();
                groups.push(ImageGroup::new(entry));
                if let Some((object, generation)) = maybe_xref {
                    xref_index.insert((object, generation), idx);
                }
            }
            ImageType::Mask => {
                if !attach_component(
                    &mut groups,
                    &xref_index,
                    info,
                    selector,
                    ComponentKind::Mask,
                ) {
                    cprintln!(
                        "<yellow>Unmatched mask on page <c,s>{}</c,s>; skipping.</yellow>",
                        page
                    );
                }
            }
            ImageType::SoftMask => {
                if !attach_component(
                    &mut groups,
                    &xref_index,
                    info,
                    selector,
                    ComponentKind::SoftMask,
                ) {
                    cprintln!(
                        "<yellow>Unmatched soft mask on page <c,s>{}</c,s>; skipping.</yellow>",
                        page
                    );
                }
            }
            _ => {
                cprintln!(
                    "<yellow>Skipping unknown image type {:?} on page <c,s>{}</c,s>.</yellow>",
                    info.image_type,
                    page
                );
            }
        }
    }

    groups
}

fn attach_component(
    groups: &mut [ImageGroup],
    xref_index: &HashMap<(i32, i32), usize>,
    info: ImageInfo,
    selector: ExtractionSelector,
    kind: ComponentKind,
) -> bool {
    let target_idx = if let Some((object, generation)) = info.xref {
        xref_index.get(&(object, generation)).copied()
    } else {
        groups
            .iter()
            .enumerate()
            .rev()
            .find(|(_, group)| match kind {
                ComponentKind::Mask => group.mask.is_none(),
                ComponentKind::SoftMask => group.soft_mask.is_none(),
            })
            .map(|(idx, _)| idx)
    };

    if let Some(idx) = target_idx {
        let target_type = match kind {
            ComponentKind::Mask => ImageExportType::Mask,
            ComponentKind::SoftMask => ImageExportType::SoftMask,
        };
        let entry = ImageEntry::new(info, selector, target_type);
        match kind {
            ComponentKind::Mask => groups[idx].mask = Some(entry),
            ComponentKind::SoftMask => groups[idx].soft_mask = Some(entry),
        }
        true
    } else {
        false
    }
}

fn export_image_entry(
    document: &mut Document,
    entry: &ImageEntry,
) -> Result<ExportedImage, String> {
    let selector = entry.selector.to_request();
    let request = ImageExportRequest {
        page_index: entry.info.page.saturating_sub(1),
        target_type: entry.target_type,
        selector,
    };

    document
        .export_image(request)
        .map_err(|err| format!("failed to extract {:?}: {err}", entry.info.image_type))
}

fn describe_component(
    page: u32,
    slot: Option<usize>,
    component_suffix: &str,
    info: &ImageInfo,
    image: &ExportedImage,
) {
    let size = format!("{} × {}", image.width, image.height);
    let comps = format!("{}c/{}bpc", image.components, image.bits_per_component);
    let dpi = format!("{:.2} × {:.2}", image.width_dpi, image.height_dpi);
    let xref = info
        .xref
        .map(|(object, generation)| format!("{} {} R", object, generation))
        .unwrap_or_else(|| "inline".into());
    let slot_fragment = slot
        .map(|idx| cformat!(" │ <bold>Group</>: #{idx:02}"))
        .unwrap_or_default();
    let component = component_label(component_suffix);

    cprintln!(
        "<bold>Page</>: {page}{slot_fragment} │ <bold>Component</>: {component} │ <bold>Type</>: {:?} │ <bold>Format</>: {:?} │ <bold>Extension</>: {:?}",
        info.image_type,
        image.format,
        image.extension,
    );
    cprintln!(
        "<bold>XRef</>: {xref} │ <bold>Dimensions</>: {size} │ <bold>Layout</>: {comps} │ <bold>DPI</>: {dpi}"
    );
    cprintln!("<bold>Payload</>: {} bytes", image.data.len());

    if let Some(globals) = image.jbig2_globals.as_ref() {
        cprintln!("<bold>JBIG2 globals</>: {} bytes captured", globals.len());
    }
    if let Some(params) = image.ccitt_params.as_ref() {
        cprintln!(
            "<bold>CCITT</>: encoding={} columns={} rows={} byte_align={} eol={} eob={} black_is_one={} damaged_rows={}",
            params.encoding,
            params.columns,
            params.rows,
            params.byte_align,
            params.end_of_line,
            params.end_of_block,
            params.black_is_one,
            params.damaged_rows_before_error,
        );
    }
}

fn persist_encoded(
    path: &Path,
    page: u32,
    slot: Option<usize>,
    component_suffix: &str,
    info: &ImageInfo,
    image: &EncodedExportedImage,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }

    fs::write(path, &image.bytes)
        .map_err(|err| format!("failed to write {}: {err}", path.display()))?;

    if let Some(globals) = image.jbig2_globals.as_ref() {
        let global_path = path.with_extension("jb2g");
        fs::write(&global_path, globals)
            .map_err(|err| format!("failed to write {}: {err}", global_path.display()))?;
    }
    if let Some(ccit_params) = image.ccitt_params.as_ref() {
        let mut params = String::new();
        if ccit_params.encoding < 0 {
            params.push_str("-4 ");
        } else if ccit_params.encoding == 0 {
            params.push_str("-1 ");
        } else {
            params.push_str("-2 ");
        }

        if ccit_params.end_of_line {
            params.push_str("-A ");
        } else {
            params.push_str("-P ");
        }

        params.push_str(&format!("-X {col}", col = ccit_params.columns));
        if ccit_params.black_is_one {
            params.push_str("-W ");
        } else {
            params.push_str("-B ");
        }
        params.push_str("-M\n"); // PDF uses MSB first
        let ccitt_params_path = path.with_extension("params");
        fs::write(&ccitt_params_path, params)
            .map_err(|err| format!("failed to write {}: {err}", ccitt_params_path.display()))?;
    }

    let slot_fragment = slot
        .map(|idx| cformat!(" image <c,s>#{} </c,s>", idx))
        .unwrap_or_default();
    let component = component_label(component_suffix);

    cprintln!(
        "Saved <m,s>{component}</m,s> (<m,s>{:?}</m,s>) page <c,s>{}</c,s>{slot_fragment} -> <m,s>{}</m,s> ({:?})",
        info.image_type,
        page,
        path.display(),
        image.extension
    );
    Ok(())
}

fn build_output_path(
    root: &Path,
    page: u32,
    slot_suffix: &str,
    component_suffix: &str,
    extension: &str,
) -> PathBuf {
    let file_name = format!("page-{page:04}{slot_suffix}{component_suffix}.{extension}");
    root.join(file_name)
}

fn export_type_from_image(kind: ImageType) -> Option<ImageExportType> {
    match kind {
        ImageType::Image => Some(ImageExportType::Image),
        ImageType::Stencil => Some(ImageExportType::Stencil),
        ImageType::Mask => Some(ImageExportType::Mask),
        ImageType::SoftMask => Some(ImageExportType::SoftMask),
        _ => None,
    }
}

fn component_label(suffix: &str) -> &'static str {
    match suffix {
        "-mask" => "mask",
        "-smask" => "soft mask",
        _ => "image",
    }
}
