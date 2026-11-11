//! Portable document exports and vector/high-resolution diagram saves.
use pecan_export::{
    DiagramFormat, DiagramInput, DocumentFormat, Snapshot, atomic_write, diagram_bytes,
    write_document,
};

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Result, ensure};
use gpui_kit::{App, AppContext, Global};

#[derive(Default)]
struct Busy(Arc<AtomicBool>);
impl Global for Busy {}
struct Permit(Arc<AtomicBool>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// One export (including its save panel) at a time bounds temporary layout memory.
fn acquire(cx: &mut App) -> Result<Permit> {
    if !cx.has_global::<Busy>() {
        cx.set_global(Busy::default());
    }
    let flag = cx.global::<Busy>().0.clone();
    ensure!(
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok(),
        "已有导出正在进行，请稍候"
    );
    Ok(Permit(flag))
}

pub(crate) fn save_document(
    snapshot: Snapshot,
    format: DocumentFormat,
    cx: &mut App,
    complete: impl FnOnce(Result<Option<PathBuf>>, &mut App) + 'static,
) {
    let client = cx.http_client();
    let directory = snapshot.directory.to_path_buf();
    let title = snapshot.title.clone();
    save(
        &title,
        &directory,
        format.extension(),
        cx,
        move |path| async move {
            write_document(snapshot, format, &path, client).await?;
            Ok(path)
        },
        complete,
    );
}

pub(crate) fn save_diagram(
    input: DiagramInput,
    format: DiagramFormat,
    cx: &mut App,
    complete: impl FnOnce(Result<Option<PathBuf>>, &mut App) + 'static,
) {
    let directory = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    save(
        "Mermaid",
        &directory,
        format.extension(),
        cx,
        move |path| async move {
            let _permit = input.slots.acquire().await;
            let bytes = diagram_bytes(&input.diagram, format, input.background)?;
            atomic_write(&path, &bytes)?;
            Ok(path)
        },
        complete,
    );
}

fn save<F, Fut>(
    title: &str,
    directory: &Path,
    extension: &'static str,
    cx: &mut App,
    work: F,
    complete: impl FnOnce(Result<Option<PathBuf>>, &mut App) + 'static,
) where
    F: FnOnce(PathBuf) -> Fut + Send + 'static,
    Fut: Future<Output = Result<PathBuf>> + Send + 'static,
{
    let permit = match acquire(cx) {
        Ok(permit) => permit,
        Err(error) => {
            complete(Err(error), cx);
            return;
        }
    };
    let stem = Path::new(title)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("文档");
    let name = format!("{stem}.{extension}");
    let origin = cx.active_window();
    let directory = if directory.is_absolute() {
        directory.to_path_buf()
    } else {
        std::env::var_os("HOME").map_or_else(
            || std::env::current_dir().unwrap_or_default(),
            |home| PathBuf::from(home).join("Documents"),
        )
    };
    let prompt = cx.prompt_for_new_path(&directory, Some(&name));
    cx.spawn(async move |cx| {
        let result = match prompt.await {
            Ok(Ok(Some(path))) => match output_path(path, extension) {
                Ok(path) => {
                    cx.background_spawn(async move { work(path).await.map(Some) })
                        .await
                }
                Err(error) => Err(error),
            },
            Ok(Ok(None)) => Ok(None),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(error.into()),
        };
        drop(permit);
        cx.update(|cx| {
            if let Some(origin) = origin {
                let note = match &result {
                    Ok(Some(path)) => Some(
                        gpui_kit::component::notification::Notification::success(format!(
                            "已导出 · {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        )),
                    ),
                    Err(error) => Some(gpui_kit::component::notification::Notification::error(
                        format!("导出失败：{error:#}"),
                    )),
                    Ok(None) => None,
                };
                if let Some(note) = note {
                    let _ = origin.update(cx, |_, window, cx| {
                        use gpui_kit::component::WindowExt as _;
                        window.push_notification(note, cx);
                    });
                }
            }
            complete(result, cx);
        });
    })
    .detach();
}

fn output_path(mut path: PathBuf, extension: &str) -> Result<PathBuf> {
    match path.extension().and_then(|suffix| suffix.to_str()) {
        Some(suffix) => ensure!(
            suffix.eq_ignore_ascii_case(extension),
            "请选择 .{extension} 后缀，避免保存成错误格式"
        ),
        None => {
            path.set_extension(extension);
            ensure!(
                !path.exists(),
                "补全后缀后文件已存在，请在保存对话框中选择完整文件名并确认替换"
            );
        }
    }
    Ok(path)
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use gpui_kit::TestAppContext;
    #[gpui_kit::test]
    fn cancelling_save_releases_the_export_slot_without_creating_a_file(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let called = Arc::new(AtomicBool::new(false));
        let done = called.clone();
        cx.update(|cx| {
            save(
                "document.md",
                directory.path(),
                "html",
                cx,
                |_| async { panic!("cancelled export must never build a document") },
                move |result, _| {
                    assert!(result.unwrap().is_none());
                    done.store(true, Ordering::Release);
                },
            );
        });
        assert!(cx.did_prompt_for_new_path());
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        assert!(called.load(Ordering::Acquire));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        cx.update(|cx| {
            assert!(
                acquire(cx).is_ok(),
                "cancellation releases the single export slot"
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagram_exports_keep_vectors_and_rasterize_at_full_resolution() {
        let diagram = pecan_mermaid::Renderer::new()
            .render(
                "flowchart LR\n A[开始] --> B[End]",
                pecan_mermaid::Appearance::Light,
            )
            .unwrap();
        assert_eq!(
            diagram_bytes(&diagram, DiagramFormat::Svg, [255; 3]).unwrap(),
            diagram.svg().as_bytes()
        );
        let bytes = diagram_bytes(&diagram, DiagramFormat::Png, [255; 3]).unwrap();
        let decoded = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let region = pecan_mermaid::Region::whole(&diagram, 2_f32.min(diagram.max_scale()));
        assert_eq!(
            (decoded.info().width, decoded.info().height),
            (region.width, region.height)
        );
    }
    #[test]
    fn saves_replace_atomically_and_match_the_selected_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.html");
        std::fs::write(&path, "old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert!(output_path(path.clone(), "pdf").is_err());
        assert_eq!(
            output_path(directory.path().join("new"), "pdf").unwrap(),
            directory.path().join("new.pdf")
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
