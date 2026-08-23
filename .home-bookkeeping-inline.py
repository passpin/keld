from pathlib import Path

path = Path("crates/keld-native-backend/src/lib.rs")
text = path.read_text(encoding="utf-8")
start_marker = "    fn emit_home_track(&mut self, register: Register) -> Result<(), BackendError> {\n"
end_marker = "    fn emit_cleanup_scope(&mut self, scope: u32, span: Span) {\n"
start = text.find(start_marker)
end = text.find(end_marker, start)
if start < 0 or end < 0:
    raise SystemExit(f"Home bookkeeping method boundaries not found: start={start} end={end}")

replacement = r'''    fn emit_home_track(&mut self, register: Register) -> Result<(), BackendError> {
        if !self.is_tracked_home(register) {
            return Ok(());
        }
        let tracker = self
            .tracker_for_register(register)
            .cloned()
            .ok_or_else(|| {
                BackendError::Unsupported("managed Home tracker is missing".to_owned())
            })?;
        let suffix = self.lines.len();
        let predecessor = self.current_label.clone();
        let count = format!("%home_track_count_{suffix}");
        let count_valid = format!("%home_track_count_valid_{suffix}");
        let scan = format!("home_track_scan_{suffix}");
        let check = format!("home_track_check_{suffix}");
        let advance = format!("home_track_advance_{suffix}");
        let append = format!("home_track_append_{suffix}");
        let store = format!("home_track_store_{suffix}");
        let done = format!("home_track_done_{suffix}");
        let index = format!("%home_track_index_{suffix}");
        let next = format!("%home_track_next_{suffix}");

        self.line(format!("{count} = load i32, ptr {}", tracker.count));
        self.line(format!(
            "{count_valid} = icmp ule i32 {count}, {}",
            tracker.capacity
        ));
        self.line(format!(
            "br i1 {count_valid}, label %{scan}, label %internal_exit"
        ));

        self.label(scan.clone());
        self.line(format!(
            "{index} = phi i32 [ 0, %{predecessor} ], [ {next}, %{advance} ]"
        ));
        let active = format!("%home_track_active_{suffix}");
        self.line(format!("{active} = icmp ult i32 {index}, {count}"));
        self.line(format!(
            "br i1 {active}, label %{check}, label %{append}"
        ));

        self.label(check.clone());
        let index64 = format!("%home_track_index64_{suffix}");
        let pointer = format!("%home_track_pointer_{suffix}");
        let current = format!("%home_track_current_{suffix}");
        let duplicate = format!("%home_track_duplicate_{suffix}");
        self.line(format!("{index64} = zext i32 {index} to i64"));
        self.line(format!(
            "{pointer} = getelementptr inbounds [{} x i32], ptr {}, i64 0, i64 {index64}",
            tracker.capacity, tracker.ids
        ));
        self.line(format!("{current} = load i32, ptr {pointer}"));
        self.line(format!(
            "{duplicate} = icmp eq i32 {current}, {}",
            register.0
        ));
        self.line(format!(
            "br i1 {duplicate}, label %{done}, label %{advance}"
        ));

        self.label(advance.clone());
        self.line(format!("{next} = add i32 {index}, 1"));
        self.line(format!("br label %{scan}"));

        self.label(append.clone());
        let has_capacity = format!("%home_track_has_capacity_{suffix}");
        self.line(format!(
            "{has_capacity} = icmp ult i32 {count}, {}",
            tracker.capacity
        ));
        self.line(format!(
            "br i1 {has_capacity}, label %{store}, label %internal_exit"
        ));

        self.label(store);
        let count64 = format!("%home_track_count64_{suffix}");
        let destination = format!("%home_track_destination_{suffix}");
        let new_count = format!("%home_track_new_count_{suffix}");
        self.line(format!("{count64} = zext i32 {count} to i64"));
        self.line(format!(
            "{destination} = getelementptr inbounds [{} x i32], ptr {}, i64 0, i64 {count64}",
            tracker.capacity, tracker.ids
        ));
        self.line(format!("store i32 {}, ptr {destination}", register.0));
        self.line(format!("{new_count} = add i32 {count}, 1"));
        self.line(format!("store i32 {new_count}, ptr {}", tracker.count));
        self.line(format!("br label %{done}"));

        self.label(done);
        Ok(())
    }

    fn emit_home_untrack(&mut self, register: Register) -> Result<(), BackendError> {
        if !self.is_tracked_home(register) {
            return Ok(());
        }
        let tracker = self
            .tracker_for_register(register)
            .cloned()
            .ok_or_else(|| {
                BackendError::Unsupported("managed Home tracker is missing".to_owned())
            })?;
        let suffix = self.lines.len();
        let predecessor = self.current_label.clone();
        let count = format!("%home_untrack_count_{suffix}");
        let count_valid = format!("%home_untrack_count_valid_{suffix}");
        let scan = format!("home_untrack_scan_{suffix}");
        let check = format!("home_untrack_check_{suffix}");
        let advance = format!("home_untrack_advance_{suffix}");
        let shift = format!("home_untrack_shift_{suffix}");
        let copy = format!("home_untrack_copy_{suffix}");
        let decrement = format!("home_untrack_decrement_{suffix}");
        let done = format!("home_untrack_done_{suffix}");
        let index = format!("%home_untrack_index_{suffix}");
        let next = format!("%home_untrack_next_{suffix}");

        self.line(format!("{count} = load i32, ptr {}", tracker.count));
        self.line(format!(
            "{count_valid} = icmp ule i32 {count}, {}",
            tracker.capacity
        ));
        self.line(format!(
            "br i1 {count_valid}, label %{scan}, label %internal_exit"
        ));

        self.label(scan.clone());
        self.line(format!(
            "{index} = phi i32 [ 0, %{predecessor} ], [ {next}, %{advance} ]"
        ));
        let active = format!("%home_untrack_active_{suffix}");
        self.line(format!("{active} = icmp ult i32 {index}, {count}"));
        self.line(format!(
            "br i1 {active}, label %{check}, label %{done}"
        ));

        self.label(check.clone());
        let index64 = format!("%home_untrack_index64_{suffix}");
        let pointer = format!("%home_untrack_pointer_{suffix}");
        let current = format!("%home_untrack_current_{suffix}");
        let found = format!("%home_untrack_found_{suffix}");
        let source_start = format!("%home_untrack_source_start_{suffix}");
        self.line(format!("{index64} = zext i32 {index} to i64"));
        self.line(format!(
            "{pointer} = getelementptr inbounds [{} x i32], ptr {}, i64 0, i64 {index64}",
            tracker.capacity, tracker.ids
        ));
        self.line(format!("{current} = load i32, ptr {pointer}"));
        self.line(format!(
            "{found} = icmp eq i32 {current}, {}",
            register.0
        ));
        self.line(format!("{source_start} = add i32 {index}, 1"));
        self.line(format!(
            "br i1 {found}, label %{shift}, label %{advance}"
        ));

        self.label(advance.clone());
        self.line(format!("{next} = add i32 {index}, 1"));
        self.line(format!("br label %{scan}"));

        self.label(shift.clone());
        let source = format!("%home_untrack_source_{suffix}");
        let source_next = format!("%home_untrack_source_next_{suffix}");
        let has_source = format!("%home_untrack_has_source_{suffix}");
        self.line(format!(
            "{source} = phi i32 [ {source_start}, %{check} ], [ {source_next}, %{copy} ]"
        ));
        self.line(format!("{has_source} = icmp ult i32 {source}, {count}"));
        self.line(format!(
            "br i1 {has_source}, label %{copy}, label %{decrement}"
        ));

        self.label(copy.clone());
        let source64 = format!("%home_untrack_source64_{suffix}");
        let source_pointer = format!("%home_untrack_source_pointer_{suffix}");
        let moved = format!("%home_untrack_moved_{suffix}");
        let destination_index = format!("%home_untrack_destination_index_{suffix}");
        let destination64 = format!("%home_untrack_destination64_{suffix}");
        let destination_pointer = format!("%home_untrack_destination_pointer_{suffix}");
        self.line(format!("{source64} = zext i32 {source} to i64"));
        self.line(format!(
            "{source_pointer} = getelementptr inbounds [{} x i32], ptr {}, i64 0, i64 {source64}",
            tracker.capacity, tracker.ids
        ));
        self.line(format!("{moved} = load i32, ptr {source_pointer}"));
        self.line(format!("{destination_index} = sub i32 {source}, 1"));
        self.line(format!(
            "{destination64} = zext i32 {destination_index} to i64"
        ));
        self.line(format!(
            "{destination_pointer} = getelementptr inbounds [{} x i32], ptr {}, i64 0, i64 {destination64}",
            tracker.capacity, tracker.ids
        ));
        self.line(format!("store i32 {moved}, ptr {destination_pointer}"));
        self.line(format!("{source_next} = add i32 {source}, 1"));
        self.line(format!("br label %{shift}"));

        self.label(decrement);
        let new_count = format!("%home_untrack_new_count_{suffix}");
        self.line(format!("{new_count} = sub i32 {count}, 1"));
        self.line(format!("store i32 {new_count}, ptr {}", tracker.count));
        self.line(format!("br label %{done}"));

        self.label(done);
        Ok(())
    }

'''

path.write_text(text[:start] + replacement + text[end:], encoding="utf-8")
