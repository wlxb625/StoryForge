use std::{
    fmt::Write as FormatWrite,
    fs::File,
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{backup::Backup, Connection, OpenFlags};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::{
    db::now,
    error::{Result, StudioError},
    guard, Studio,
};

const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;
const WORD_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";

fn persistent_error(problem: impl std::fmt::Display) -> StudioError {
    StudioError::new(500, format!("无法完成导出：{problem}"))
}

fn source_project(path: &Path) -> Result<Connection> {
    let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(Duration::from_secs(5))?;
    Ok(source)
}

fn inspect_bytes(bytes: &[u8], credentials: &[String]) -> Result<()> {
    let known = credentials.iter().any(|secret| {
        !secret.is_empty()
            && bytes
                .windows(secret.len())
                .any(|fragment| fragment == secret.as_bytes())
    });
    let prefixes = [b"sk-".as_slice(), b"xai-".as_slice(), b"Bearer ".as_slice()];
    let inferred = bytes.iter().enumerate().any(|(index, value)| {
        matches!(*value, b's' | b'x' | b'B')
            && prefixes.iter().any(|prefix| {
                bytes[index..]
                    .strip_prefix(*prefix)
                    .and_then(|tail| tail.get(..24))
                    .is_some_and(|token| {
                        token
                            .iter()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'-'))
                    })
            })
    });
    if known || inferred {
        return Err(StudioError::new(
            409,
            "检测到疑似 API 凭据内容。请检查工程正文或历史记录后再分享。",
        ));
    }
    Ok(())
}

fn scan_for_credentials(path: &Path, credentials: &[String]) -> Result<()> {
    let mut input = BufReader::new(File::open(path)?);
    let mut carry = Vec::<u8>::new();
    let keep = credentials
        .iter()
        .map(String::len)
        .max()
        .unwrap_or(0)
        .max(90);
    let mut chunk = [0; 65_536];
    loop {
        let size = input.read(&mut chunk)?;
        if size == 0 {
            return Ok(());
        }
        carry.extend_from_slice(&chunk[..size]);
        inspect_bytes(&carry, credentials)?;
        if carry.len() > keep {
            carry.drain(..carry.len() - keep);
        }
    }
}

fn scan_project_fields(db: &Connection, credentials: &[String]) -> Result<()> {
    use rusqlite::types::ValueRef;
    let names = db
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'chapters_fts%'
        AND name NOT LIKE 'sqlite_%'",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for table in names {
        let mut statement =
            db.prepare(&format!("SELECT * FROM \"{}\"", table.replace('"', "\"\"")))?;
        let columns = statement.column_count();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            for column in 0..columns {
                if let ValueRef::Text(value) | ValueRef::Blob(value) = row.get_ref(column)? {
                    inspect_bytes(value, credentials)?;
                }
            }
        }
    }
    Ok(())
}

fn snapshot(source: &Connection, output: &Path, credentials: &[String]) -> Result<()> {
    let mut target = Connection::open(output)?;
    {
        let backup = Backup::new(source, &mut target)?;
        backup.run_to_completion(128, Duration::from_millis(12), None)?;
    }
    target.execute_batch("PRAGMA journal_mode=DELETE; VACUUM;")?;
    let integrity: String = target.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(StudioError::new(500, "工程快照未通过完整性检查"));
    }
    scan_project_fields(&target, credentials)?;
    drop(target);
    scan_for_credentials(output, credentials)
}

struct Manuscript {
    id: String,
    title: String,
    chapters: Vec<(String, String)>,
}

fn load_manuscript(source: &mut Connection) -> Result<Manuscript> {
    let tx = source.transaction()?;
    let (id, title) = tx.query_row("SELECT project_id,title FROM project_meta", [], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    let chapters = {
        let mut query = tx.prepare(
            "SELECT c.title,r.content FROM chapters c
            JOIN chapter_revisions r ON r.revision_id=c.current_revision_id
            WHERE c.status!='archived' ORDER BY c.chapter_order",
        )?;
        let rows = query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    tx.commit()?;
    if chapters.is_empty() {
        return Err(StudioError::new(400, "请先保存至少一章正文"));
    }
    Ok(Manuscript {
        id,
        title,
        chapters,
    })
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{9}'
            | '\u{a}'
            | '\u{d}'
            | '\u{20}'..='\u{d7ff}'
            | '\u{e000}'..='\u{fffd}'
            | '\u{10000}'..='\u{10ffff}' => out.push(ch),
            _ => out.push('\u{fffd}'),
        }
    }
    out
}

fn entry(
    archive: &mut ZipWriter<File>,
    name: &str,
    options: SimpleFileOptions,
    contents: &str,
) -> Result<()> {
    archive
        .start_file(name, options)
        .map_err(persistent_error)?;
    archive.write_all(contents.as_bytes())?;
    Ok(())
}

fn paragraph(text: &str, style: &str) -> String {
    let props = if style.is_empty() {
        String::new()
    } else {
        format!(r#"<w:pPr><w:pStyle w:val="{style}"/></w:pPr>"#)
    };
    format!(
        r#"<w:p>{props}<w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
        escape(text)
    )
}

fn write_docx(file: File, book: &Manuscript) -> Result<()> {
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    entry(
        &mut zip,
        "[Content_Types].xml",
        options,
        &format!(
            r#"{XML}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
      <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>
      <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
      <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#
        ),
    )?;
    entry(
        &mut zip,
        "_rels/.rels",
        options,
        &format!(
            r#"{XML}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
      <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#
        ),
    )?;
    entry(
        &mut zip,
        "word/_rels/document.xml.rels",
        options,
        &format!(
            r#"{XML}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
      <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#
        ),
    )?;
    entry(
        &mut zip,
        "word/styles.xml",
        options,
        &format!(
            r#"{XML}<w:styles xmlns:w="{WORD_NS}">
      <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Georgia" w:eastAsia="SimSun"/><w:sz w:val="23"/><w:lang w:eastAsia="zh-CN"/></w:rPr></w:rPrDefault>
      <w:pPrDefault><w:pPr><w:spacing w:after="100" w:line="340" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>
      <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
      <w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/>
      <w:pPr><w:spacing w:before="360" w:after="320"/><w:jc w:val="center"/></w:pPr><w:rPr><w:b/><w:sz w:val="46"/></w:rPr></w:style>
      <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/>
      <w:pPr><w:keepNext/><w:pageBreakBefore/><w:spacing w:after="280"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="34"/></w:rPr></w:style></w:styles>"#
        ),
    )?;
    let mut body = paragraph(&book.title, "Title");
    for (title, content) in &book.chapters {
        body.push_str(&paragraph(title, "Heading1"));
        for line in content.split('\n') {
            body.push_str(&paragraph(line.trim_end_matches('\r'), ""));
        }
    }
    entry(
        &mut zip,
        "word/document.xml",
        options,
        &format!(
            r#"{XML}<w:document xmlns:w="{WORD_NS}"><w:body>{body}
        <w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"
        w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
        ),
    )?;
    zip.finish().map_err(persistent_error)?;
    Ok(())
}

fn write_epub(file: File, book: &Manuscript) -> Result<()> {
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    entry(
        &mut zip,
        "mimetype",
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        "application/epub+zip",
    )?;
    entry(
        &mut zip,
        "META-INF/container.xml",
        options,
        &format!(
            r#"{XML}<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
        <rootfiles><rootfile full-path="EPUB/book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#
        ),
    )?;
    entry(&mut zip,"EPUB/style.css",options,"body{font-family:serif;line-height:1.7;margin:5%;} p{white-space:pre-wrap;margin:.62em 0;} h1{margin:1.1em 0;line-height:1.3;}")?;
    let mut manifest = String::new();
    let mut spine = String::new();
    let mut contents = String::new();
    for (index, (title, content)) in book.chapters.iter().enumerate() {
        let file = format!("chapter-{}.xhtml", index + 1);
        let chapter = format!("c{}", index + 1);
        let title = escape(title);
        write!(
            manifest,
            r#"<item id="{chapter}" href="{file}" media-type="application/xhtml+xml"/>"#
        )
        .expect("string");
        write!(spine, r#"<itemref idref="{chapter}"/>"#).expect("string");
        write!(contents, r#"<li><a href="{file}">{title}</a></li>"#).expect("string");
        let text = content
            .split('\n')
            .map(|line| {
                let clean = line.trim_end_matches('\r');
                if clean.is_empty() {
                    "<p><br /></p>".into()
                } else {
                    format!("<p>{}</p>", escape(clean))
                }
            })
            .collect::<String>();
        entry(
            &mut zip,
            &format!("EPUB/{file}"),
            options,
            &format!(
                r#"{XML}<html xmlns="{XHTML_NS}" xml:lang="zh-CN" lang="zh-CN"><head><title>{title}</title>
                <link rel="stylesheet" href="style.css" type="text/css"/></head><body><h1>{title}</h1>{text}</body></html>"#
            ),
        )?;
    }
    entry(
        &mut zip,
        "EPUB/nav.xhtml",
        options,
        &format!(
            r#"{XML}<html xmlns="{XHTML_NS}" xmlns:epub="http://www.idpf.org/2007/ops"
        xml:lang="zh-CN" lang="zh-CN"><head><title>目录</title></head><body><nav epub:type="toc" id="toc"><h1>目录</h1><ol>{contents}</ol></nav></body></html>"#
        ),
    )?;
    let title = escape(&book.title);
    let id = escape(&book.id);
    let modified = now()
        .replace("Z", "")
        .split('.')
        .next()
        .unwrap_or_default()
        .to_owned();
    entry(
        &mut zip,
        "EPUB/book.opf",
        options,
        &format!(
            r#"{XML}<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id" xml:lang="zh-CN">
      <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="book-id">urn:uuid:{id}</dc:identifier>
      <dc:title>{title}</dc:title><dc:language>zh-CN</dc:language><meta property="dcterms:modified">{modified}Z</meta></metadata>
      <manifest><item id="navigation" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
      <item id="style" href="style.css" media-type="text/css"/>{manifest}</manifest><spine>{spine}</spine></package>"#
        ),
    )?;
    zip.finish().map_err(persistent_error)?;
    Ok(())
}

fn export_file(source: &Path, output: &Path, kind: &str, credentials: &[String]) -> Result<()> {
    let directory = output
        .parent()
        .ok_or_else(|| StudioError::new(400, "请选择输出目录"))?;
    let temp = tempfile::NamedTempFile::new_in(directory)?;
    let mut connection = source_project(source)?;
    match kind {
        "story" => snapshot(&connection, temp.path(), credentials)?,
        "docx" => write_docx(temp.reopen()?, &load_manuscript(&mut connection)?)?,
        "epub" => write_epub(temp.reopen()?, &load_manuscript(&mut connection)?)?,
        _ => return Err(StudioError::new(400, "无法识别导出格式")),
    }
    persist_export(temp, output)
}

fn persist_export(temp: tempfile::NamedTempFile, output: &Path) -> Result<()> {
    temp.as_file().sync_all()?;
    temp.persist_noclobber(output).map_err(|err| {
        if err.error.kind() == std::io::ErrorKind::AlreadyExists {
            StudioError::new(409, "目标文件已存在，请选择一个新文件名")
        } else {
            persistent_error(err.error)
        }
    })?;
    Ok(())
}

fn local_credentials(studio: &Studio) -> Result<Vec<String>> {
    guard(&studio.profiles)?
        .list()?
        .into_iter()
        .filter(|listed| listed.has_key)
        .map(|listed| listed.profile.credential_for_request())
        .collect::<Result<Vec<_>>>()
        .map(|entries| entries.into_iter().flatten().collect())
}

#[tauri::command]
pub async fn export_pack(app: AppHandle, name: String, contents: String) -> Result<Option<String>> {
    let suffix = name
        .rsplit_once('.')
        .filter(|(stem, suffix)| {
            !stem.is_empty()
                && stem.len() <= 120
                && stem
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                && matches!(*suffix, "md" | "json")
        })
        .map(|(_, suffix)| suffix.to_owned())
        .ok_or_else(|| StudioError::new(400, "配方文件名无效"))?;
    if contents.is_empty() || contents.len() > 600_000 {
        return Err(StudioError::new(400, "配方内容超过导出限制"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let studio = app.state::<Studio>();
        inspect_bytes(contents.as_bytes(), &local_credentials(&studio)?)?;
        let Some(destination) = app
            .dialog()
            .file()
            .set_title("导出配方")
            .add_filter("声明式配方", &[suffix.as_str()])
            .set_file_name(name)
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let mut output = destination
            .into_path()
            .map_err(|_| StudioError::new(400, "请选择本机文件"))?;
        if !output
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(&suffix))
        {
            output.set_extension(suffix);
        }
        let directory = output
            .parent()
            .ok_or_else(|| StudioError::new(400, "请选择输出目录"))?;
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        temp.write_all(contents.as_bytes())?;
        persist_export(temp, &output)?;
        Ok(Some(output.to_string_lossy().into()))
    })
    .await
    .map_err(persistent_error)?
}

#[tauri::command]
pub async fn export_project(app: AppHandle, format: String) -> Result<Option<String>> {
    let (filter, suffix) = match format.as_str() {
        "story" => ("故事工程快照", "story"),
        "docx" => ("Word 成稿", "docx"),
        "epub" => ("EPUB 电子书", "epub"),
        _ => return Err(StudioError::new(400, "无法识别导出格式")),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let studio = app.state::<Studio>();
        let info = studio.project(|project| Ok(project.info.clone()))?;
        let safe_title = info
            .title
            .chars()
            .map(|ch| if r#"<>:"/\|?*"#.contains(ch) { '_' } else { ch })
            .collect::<String>();
        let ending = if suffix == "story" { "-快照" } else { "" };
        let Some(destination) = app
            .dialog()
            .file()
            .set_title("导出 StoryForge")
            .add_filter(filter, &[suffix])
            .set_file_name(format!("{safe_title}{ending}.{suffix}"))
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let mut output = destination
            .into_path()
            .map_err(|_| StudioError::new(400, "请选择本机文件"))?;
        if !output
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(suffix))
        {
            output.set_extension(suffix);
        }
        let keys = if suffix == "story" {
            local_credentials(&studio)?
        } else {
            Vec::new()
        };
        export_file(&PathBuf::from(info.path), &output, &format, &keys)?;
        Ok(Some(output.to_string_lossy().into()))
    })
    .await
    .map_err(persistent_error)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Project, RevisionInput};

    fn zip_text(path: &Path, name: &str) -> String {
        let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        let mut text = String::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    #[test]
    fn export_snapshot_docx_and_epub_preserve_confirmed_text_and_block_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("小说.story");
        let mut project = Project::create(source.clone(), "雾 & 灯").unwrap();
        let chapter = project.chapters().unwrap().remove(0);
        let write = |base: Option<String>, source: &str, text: &str| RevisionInput {
            chapter_id: chapter.chapter_id.clone(),
            base_revision_id: base,
            source: source.into(),
            content: text.into(),
            draft_task_id: None,
            edit_level: None,
            canon_confirmed: false,
        };
        let revision = project
            .save_revision(write(None, "manual_edit", "第一行 <雨>。\n\n第二行 & 门。"))
            .unwrap();
        project
            .save_revision(write(
                Some(revision.revision_id.clone()),
                "auto_buffer",
                "不能进入成稿的临时稿",
            ))
            .unwrap();
        let saved = dir.path().join("快照.story");
        export_file(&source, &saved, "story", &[]).unwrap();
        let reopened = Project::open(saved.clone()).unwrap();
        assert_eq!(
            reopened.chapters().unwrap()[0]
                .current_revision_id
                .as_deref(),
            Some(revision.revision_id.as_str())
        );
        assert_eq!(
            export_file(&source, &saved, "story", &[])
                .unwrap_err()
                .status,
            409
        );
        let word = dir.path().join("书.docx");
        let epub = dir.path().join("书.epub");
        export_file(&source, &word, "docx", &[]).unwrap();
        export_file(&source, &epub, "epub", &[]).unwrap();
        assert!(zip_text(&word, "word/document.xml").contains("第一行 &lt;雨&gt;。"));
        assert!(!zip_text(&word, "word/document.xml").contains("临时稿"));
        assert!(zip_text(&epub, "EPUB/chapter-1.xhtml").contains("第二行 &amp; 门。"));
        assert!(zip_text(&epub, "EPUB/nav.xhtml").contains("chapter-1.xhtml"));
        let mut archive = zip::ZipArchive::new(File::open(epub).unwrap()).unwrap();
        let first = archive.by_index(0).unwrap();
        assert_eq!(first.name(), "mimetype");
        assert_eq!(first.compression(), CompressionMethod::Stored);
        let mock = format!("{}{}", "sk-", "fake".repeat(9));
        project
            .save_revision(write(Some(revision.revision_id), "manual_edit", &mock))
            .unwrap();
        let blocked = dir.path().join("禁止.story");
        assert_eq!(
            export_file(&source, &blocked, "story", &[mock])
                .unwrap_err()
                .status,
            409
        );
        assert!(!blocked.exists());
    }
}
