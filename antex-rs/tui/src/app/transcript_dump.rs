use std::fmt::Write as _;
use std::sync::Arc;

use pulldown_cmark::Event;
use pulldown_cmark::HeadingLevel;
use pulldown_cmark::Options;
use pulldown_cmark::Parser;
use pulldown_cmark::Tag;
use pulldown_cmark::TagEnd;

use super::App;
use super::transcript_export::TranscriptBlock;
use super::transcript_export::TranscriptBlockKind;
use super::transcript_export::load_export_transcript;
use super::transcript_export::transcript_blocks;
use super::transcript_export::write_transcript;
use crate::app_server_session::AppServerSession;
use crate::history_cell::HistoryCell;
use crate::thread_transcript::RawReasoningVisibility;

impl App {
    pub(super) async fn dump_transcript_html(
        &mut self,
        app_server: &mut AppServerSession,
    ) -> Result<(), String> {
        let thread_id = self
            .chat_widget
            .thread_id()
            .ok_or_else(|| "No active conversation to dump.".to_string())?;
        let visibility = if self.config.show_raw_agent_reasoning {
            RawReasoningVisibility::Visible
        } else {
            RawReasoningVisibility::Hidden
        };
        let cells = load_export_transcript(
            app_server,
            thread_id,
            visibility,
            Some(&self.config),
            self.transcript_cells.clone(),
        )
        .await?;
        let exported_at = chrono::Utc::now();
        let html = render_html_transcript(&cells, &thread_id.to_string(), exported_at)?;
        let short_id = thread_id.to_string().chars().take(8).collect::<String>();
        let filename = format!(
            "codex-conversation-{short_id}-{}.html",
            exported_at.format("%Y%m%d-%H%M%S")
        );
        let cwd = if self.app_server_target.uses_remote_workspace() {
            self.launch_cwd.as_path()
        } else {
            self.chat_widget.config_ref().cwd.as_path()
        };
        let path = write_transcript(cwd, filename.as_ref(), &html)?;
        self.chat_widget.add_info_message(
            format!("Saved HTML conversation to {}", path.display()),
            /*hint*/ None,
        );
        Ok(())
    }
}

fn render_html_transcript(
    cells: &[Arc<dyn HistoryCell>],
    thread_id: &str,
    exported_at: chrono::DateTime<chrono::Utc>,
) -> Result<String, String> {
    let blocks = transcript_blocks(cells)?;
    let mut body = String::new();
    let mut activities = Vec::new();
    for block in blocks {
        match block.kind {
            TranscriptBlockKind::User => {
                push_activities(&mut body, &mut activities);
                push_message(&mut body, "you", "You", &block.markdown);
            }
            TranscriptBlockKind::Assistant => {
                push_activities(&mut body, &mut activities);
                push_message(&mut body, "codex", "Antex", &block.markdown);
            }
            TranscriptBlockKind::Plan => {
                push_activities(&mut body, &mut activities);
                push_message(&mut body, "codex", "Antex · plan", &block.markdown);
            }
            TranscriptBlockKind::Reasoning | TranscriptBlockKind::Activity => {
                activities.push(block);
            }
        }
    }
    push_activities(&mut body, &mut activities);
    let thread_id = escape_html(thread_id);
    let exported_at = exported_at.format("%Y-%m-%d %H:%M UTC");
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Antex conversation</title>
<style>
@font-face {{ font-family:"CourierCyr"; src:url("https://cdn.jsdelivr.net/gh/alchemmist/personal-site@main/site/static/assets/fonts/couriercyrps.woff2") format("woff2"); font-display:swap; }}
@font-face {{ font-family:"Computer Modern Roman"; src:url("https://cdn.jsdelivr.net/gh/alchemmist/personal-site@main/site/static/assets/fonts/cmu.serif-roman.woff2") format("woff2"); font-display:swap; }}
@font-face {{ font-family:"Computer Modern Roman"; src:url("https://cdn.jsdelivr.net/gh/alchemmist/personal-site@main/site/static/assets/fonts/cmu.serif-bold.woff2") format("woff2"); font-weight:bold; font-display:swap; }}
@font-face {{ font-family:"Computer Modern Typewriter"; src:url("https://cdn.jsdelivr.net/gh/alchemmist/personal-site@main/site/static/assets/fonts/cmu.typewriter-text-regular.woff2") format("woff2"); font-display:swap; }}
:root {{ --ink:#000; --muted:rgba(0,0,0,.5); --line:#ddd; --soft:rgba(0,0,0,.02); }}
* {{ box-sizing:border-box; }}
html {{ background:#fafafa; }}
body {{ min-height:100vh; margin:0; background:#fff; color:var(--ink); font:18px/1.5 "Computer Modern Roman",serif; }}
main {{ width:50%; min-width:700px; margin:0 auto; padding:3em 0 6em; }}
header.page {{ padding-bottom:1em; border-bottom:1px solid var(--line); text-align:center; }}
h1 {{ margin:10px 10px 0; font-size:25px; line-height:1.2; }}
.meta,.role,summary,.action-label {{ font-family:"CourierCyr",monospace; }}
.meta {{ color:var(--muted); font-size:15px; overflow-wrap:anywhere; }}
.message {{ padding:1.5em 0; border-bottom:1px solid var(--line); }}
.message.you {{ border-left:4px solid #aaa; padding-left:1em; }}
.role {{ margin-bottom:.5em; color:var(--muted); font-size:15px; }}
.content {{ min-width:0; overflow-wrap:anywhere; }}
.content > :first-child {{ margin-top:0; }} .content > :last-child {{ margin-bottom:0; }}
p,ul,ol,pre,blockquote,table {{ margin:0 0 .5em; }}
ul,ol {{ padding-left:1.5em; }}
h2,h3,h4 {{ margin:1.3em 0 .7em; line-height:1.2; }}
pre {{ max-width:100%; margin:.5rem 0; padding:1rem; overflow-x:auto; border-radius:.3em; background:var(--soft); box-shadow:0 1px 4px rgba(0,0,0,.06); white-space:pre-wrap; overflow-wrap:anywhere; }}
pre,code {{ font-family:"Computer Modern Typewriter",monospace; }}
pre {{ font-size:.96em; line-height:1.2; text-align:left; }}
:not(pre)>code {{ padding:.1em .3em; border-radius:3px; background:rgba(0,0,0,.04); color:#333; font-size:.9em; }}
blockquote {{ margin:.5em 0; padding:.5em 1em; border-left:4px solid #aaa; color:#333; }}
table {{ display:block; max-width:100%; overflow-x:auto; border-collapse:collapse; font-size:.9em; }} th,td {{ padding:8px; border-bottom:1px solid var(--line); text-align:left; }}
.actions {{ padding:.8em 0; border-bottom:1px solid var(--line); }}
summary {{ cursor:pointer; color:var(--muted); font-size:15px; list-style:none; }} summary::-webkit-details-marker {{ display:none; }}
summary::before {{ content:"+ "; }} details[open] summary::before {{ content:"− "; }}
.action {{ min-width:0; margin-top:.7em; padding:1em; border-radius:.3em; background:var(--soft); box-shadow:0 1px 4px rgba(0,0,0,.06); }}
.action-label {{ margin-bottom:.5em; color:var(--muted); font-size:13px; }}
.action pre {{ margin:0; padding:0; border:0; background:transparent; white-space:pre-wrap; overflow-wrap:anywhere; }}
@media (max-width:650px) {{ main {{ width:100%; min-width:0; padding:1rem; }} body {{ font-size:16px; }} h1 {{ font-size:20px; }} .message.you {{ padding-left:.8em; }} pre {{ font-size:.85em; }} }}
</style>
</head>
<body>
<main>
<header class="page"><h1>Antex conversation</h1><div class="meta">{thread_id} · {exported_at}</div></header>
{body}</main>
</body>
</html>
"#
    ))
}

fn push_message(body: &mut String, class: &str, role: &str, markdown: &str) {
    let mut rendered = String::new();
    render_safe_markdown(markdown, &mut rendered);
    let _ = writeln!(
        body,
        "<section class=\"message {class}\"><div class=\"role\">{role}</div><div class=\"content\">{rendered}</div></section>"
    );
}

fn push_activities(body: &mut String, activities: &mut Vec<TranscriptBlock>) {
    if activities.is_empty() {
        return;
    }
    let count = activities.len();
    let _ = writeln!(
        body,
        "<details class=\"actions\"><summary>{count} {}</summary>",
        if count == 1 { "action" } else { "actions" }
    );
    for activity in activities.drain(..) {
        let label = match activity.kind {
            TranscriptBlockKind::Reasoning => "Reasoning",
            TranscriptBlockKind::Activity => "Action",
            TranscriptBlockKind::User
            | TranscriptBlockKind::Assistant
            | TranscriptBlockKind::Plan => continue,
        };
        let _ = writeln!(
            body,
            "<div class=\"action\"><div class=\"action-label\">{label}</div><pre>{}</pre></div>",
            escape_html(&activity.markdown)
        );
    }
    body.push_str("</details>\n");
}

fn render_safe_markdown(markdown: &str, output: &mut String) {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => push_start_tag(output, tag),
            Event::End(tag) => push_end_tag(output, tag),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                output.push_str(&escape_html(&text));
            }
            Event::Code(code) => {
                let _ = write!(output, "<code>{}</code>", escape_html(&code));
            }
            Event::FootnoteReference(label) => {
                let _ = write!(output, "<sup>[{}]</sup>", escape_html(&label));
            }
            Event::SoftBreak => output.push('\n'),
            Event::HardBreak => output.push_str("<br>\n"),
            Event::Rule => output.push_str("<hr>\n"),
            Event::TaskListMarker(checked) => {
                output.push_str(if checked { "☒ " } else { "☐ " });
            }
        }
    }
}

fn push_start_tag(output: &mut String, tag: Tag<'_>) {
    match tag {
        Tag::Paragraph => output.push_str("<p>"),
        Tag::Heading { level, .. } => {
            let level = heading_number(level);
            let _ = write!(output, "<h{level}>");
        }
        Tag::BlockQuote => output.push_str("<blockquote>"),
        Tag::CodeBlock(_) => output.push_str("<pre><code>"),
        Tag::HtmlBlock | Tag::Link { .. } | Tag::Image { .. } | Tag::MetadataBlock(_) => {}
        Tag::List(Some(start)) => {
            let _ = write!(output, "<ol start=\"{start}\">");
        }
        Tag::List(None) => output.push_str("<ul>"),
        Tag::Item => output.push_str("<li>"),
        Tag::FootnoteDefinition(label) => {
            let _ = write!(
                output,
                "<aside class=\"footnote\"><sup>{}</sup> ",
                escape_html(&label)
            );
        }
        Tag::Table(_) => output.push_str("<table>"),
        Tag::TableHead => output.push_str("<thead>"),
        Tag::TableRow => output.push_str("<tr>"),
        Tag::TableCell => output.push_str("<td>"),
        Tag::Emphasis => output.push_str("<em>"),
        Tag::Strong => output.push_str("<strong>"),
        Tag::Strikethrough => output.push_str("<del>"),
    }
}

fn push_end_tag(output: &mut String, tag: TagEnd) {
    match tag {
        TagEnd::Paragraph => output.push_str("</p>\n"),
        TagEnd::Heading(level) => {
            let level = heading_number(level);
            let _ = writeln!(output, "</h{level}>");
        }
        TagEnd::BlockQuote => output.push_str("</blockquote>\n"),
        TagEnd::CodeBlock => output.push_str("</code></pre>\n"),
        TagEnd::HtmlBlock | TagEnd::Link | TagEnd::Image | TagEnd::MetadataBlock(_) => {}
        TagEnd::List(true) => output.push_str("</ol>\n"),
        TagEnd::List(false) => output.push_str("</ul>\n"),
        TagEnd::Item => output.push_str("</li>\n"),
        TagEnd::FootnoteDefinition => output.push_str("</aside>\n"),
        TagEnd::Table => output.push_str("</table>\n"),
        TagEnd::TableHead => output.push_str("</thead>"),
        TagEnd::TableRow => output.push_str("</tr>\n"),
        TagEnd::TableCell => output.push_str("</td>"),
        TagEnd::Emphasis => output.push_str("</em>"),
        TagEnd::Strong => output.push_str("</strong>"),
        TagEnd::Strikethrough => output.push_str("</del>"),
    }
}

fn heading_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn escape_html(text: &str) -> String {
    text.chars().fold(
        String::with_capacity(text.len()),
        |mut escaped, character| {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&#39;"),
                _ => escaped.push(character),
            }
            escaped
        },
    )
}

#[cfg(test)]
#[path = "transcript_dump_tests.rs"]
mod tests;
