mod common;

use common::*;

#[test]
fn addition_and_undelete_only_have_right_side() {
    assert!(comment_anchor::validate("left", 1, 1, "new\n", Some(&json!("add"))).is_err());
    assert!(comment_anchor::validate("right", 1, 1, "new\n", Some(&json!("add, edit"))).is_ok());
    assert!(
        comment_anchor::validate("left", 1, 1, "restored", Some(&json!("undelete, delete")))
            .is_err()
    );
    assert!(comment_anchor::validate("right", 1, 1, "restored", Some(&json!("undelete"))).is_ok())
}
#[test]
fn deletion_only_has_left_side() {
    assert!(comment_anchor::validate("right", 1, 1, "old", Some(&json!("delete"))).is_err());
    assert!(comment_anchor::validate("left", 1, 1, "old", Some(&json!("delete"))).is_ok())
}
#[test]
fn numeric_azure_change_flags_are_supported() {
    assert!(comment_anchor::validate_side("left", Some(&json!(1))).is_err());
    assert!(comment_anchor::validate_side("right", Some(&json!(16))).is_err());
    assert!(comment_anchor::validate_side("right", Some(&json!(32))).is_ok());
    assert!(comment_anchor::validate_side("left", Some(&json!(32))).is_err())
}
#[test]
fn edited_files_have_both_sides_and_missing_metadata_fails_closed() {
    assert!(comment_anchor::validate_side("left", Some(&json!("edit"))).is_ok());
    assert!(comment_anchor::validate_side("right", Some(&json!("edit"))).is_ok());
    assert!(comment_anchor::validate_side("right", None).is_err())
}
#[test]
fn line_count_does_not_invent_line_after_terminal_newline() {
    assert_eq!(comment_anchor::line_count(""), 0);
    assert_eq!(comment_anchor::line_count("one"), 1);
    assert_eq!(comment_anchor::line_count("one\n"), 1);
    assert_eq!(comment_anchor::line_count("one\ntwo"), 2);
    assert_eq!(comment_anchor::line_count("one\ntwo\n"), 2);
    assert!(comment_anchor::validate("right", 1, 2, "one\ntwo\n", Some(&json!("edit"))).is_ok());
    assert!(comment_anchor::validate("right", 2, 3, "one\ntwo\n", Some(&json!("edit"))).is_err())
}
#[test]
fn empty_file_has_no_valid_positive_anchor() {
    assert!(comment_anchor::validate("right", 1, 1, "", Some(&json!("add"))).is_err())
}
