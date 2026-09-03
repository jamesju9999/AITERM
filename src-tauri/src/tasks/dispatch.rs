//! Turning a queued card into a live dispatch: compose the prompt text,
//! spawn a visible PTY tab running the configured `claude` command, wait for
//! it to settle, then type the prompt in using the same CR-terminated /
//! done-marker-instruction sequencing `coordination_ops::send_input` uses.

/// Body text plus, if any attachments, one trailing line pointing `claude` at
/// their on-disk paths (they've already been copied into the task dir).
pub fn build_prompt(body: &str, attachment_paths: &[String]) -> String {
    if attachment_paths.is_empty() {
        return body.to_string();
    }
    let list = attachment_paths.join("、");
    format!("{body}\n\n（相關附件：{list}）")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_just_the_body_when_there_are_no_attachments() {
        assert_eq!(build_prompt("Do the thing", &[]), "Do the thing");
    }

    #[test]
    fn prompt_appends_one_line_listing_attachment_paths() {
        let p = build_prompt(
            "Refactor per the spec",
            &["/data/tasks/x/attachments/spec.md".into(), "/data/tasks/x/attachments/before.png".into()],
        );
        assert!(p.starts_with("Refactor per the spec"));
        assert!(p.contains("/data/tasks/x/attachments/spec.md"));
        assert!(p.contains("/data/tasks/x/attachments/before.png"));
        // Attachment note is on its own line, after a blank line.
        assert!(p.contains("\n\n"));
    }

    #[test]
    fn blank_body_still_produces_the_attachment_note() {
        let p = build_prompt("", &["/a/b.txt".into()]);
        assert!(p.contains("/a/b.txt"));
    }
}
