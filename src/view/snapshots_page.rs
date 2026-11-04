use crate::model::{
    flow8::{FLOW8Controller, RenameTarget, SNAPSHOT_COUNT},
    message::InterfaceMessage,
};
use iced::{
    widget::{button, column, container, row, text, text_input, Space},
    Center, Element, Fill,
};
const GRID_COLS: usize = 4;

const EMPTY_NUM_COLOR: iced::Color = iced::Color {
    r: 0.45,
    g: 0.45,
    b: 0.45,
    a: 1.0,
};

const AVAILABLE_NUM_COLOR: iced::Color = iced::Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

const AVAILABLE_NAME_COLOR: iced::Color = iced::Color {
    r: 0.85,
    g: 0.85,
    b: 0.85,
    a: 1.0,
};

/// Snapshot cards live in the Settings page (no top-bar tab) — recall by
/// clicking a card, save/rename/delete under each card.
pub fn snapshots_section(controller: &FLOW8Controller) -> Element<'_, InterfaceMessage> {
    let mut rows: Vec<Element<'_, InterfaceMessage>> = Vec::new();

    for chunk_start in (0..SNAPSHOT_COUNT).step_by(GRID_COLS) {
        let mut row_items: Vec<Element<'_, InterfaceMessage>> = Vec::new();
        for i in chunk_start..((chunk_start + GRID_COLS).min(SNAPSHOT_COUNT)) {
            let snapshot_num = i + 1;
            let name = controller
                .snapshot_names
                .get(i)
                .and_then(|n| n.as_ref());

            let has_name = name.is_some();

            let editing_rename = controller.rename_target == Some(RenameTarget::Snapshot(i));
            let editing_save = controller.rename_target == Some(RenameTarget::SnapshotSave(i));
            let editing = editing_rename || editing_save;

            let btn_content = match name {
                Some(n) => column![
                    text(format!("{:02}", snapshot_num))
                        .size(20)
                        .align_x(Center)
                        .color(AVAILABLE_NUM_COLOR),
                    text(n.as_str())
                        .size(15)
                        .align_x(Center)
                        .color(AVAILABLE_NAME_COLOR),
                ]
                .align_x(Center)
                .spacing(4),
                None => column![
                    text(format!("{:02}", snapshot_num))
                        .size(20)
                        .align_x(Center)
                        .color(EMPTY_NUM_COLOR),
                    text(" ").size(15),
                ]
                .align_x(Center)
                .spacing(4),
            };

            let btn = button(btn_content)
                .on_press(InterfaceMessage::LoadSnapshot(i as u8))
                .padding([16, 16])
                .width(Fill)
                .style(if has_name { button::primary } else { button::secondary });

            // below the card: ✎ rename + Save + Del, or the inline editor while editing
            let delete_pending = controller
                .delete_pending
                .map(|(s, _)| s == i)
                .unwrap_or(false);
            let below: Element<'_, InterfaceMessage> = if editing {
                let editor_label: &str = if editing_save { "save as" } else { "rename" };
                column![
                    text_input(editor_label, &controller.rename_buffer)
                        .on_input(InterfaceMessage::RenameChanged)
                        .on_submit(InterfaceMessage::RenameSubmit)
                        .size(12)
                        .width(Fill),
                    row![
                        button(text("\u{2713}").size(11))
                            .on_press(InterfaceMessage::RenameSubmit)
                            .padding([2, 8])
                            .style(button::success),
                        button(text("\u{2715}").size(11))
                            .on_press(InterfaceMessage::RenameCancel)
                            .padding([2, 8])
                            .style(button::secondary),
                    ]
                    .spacing(4),
                ]
                .align_x(Center)
                .spacing(4)
                .width(Fill)
                .into()
            } else {
                row![
                    button(text("Rename").size(10))
                        .on_press(InterfaceMessage::RenameStartSnapshot(i))
                        .padding([3, 10])
                        .style(button::secondary),
                    button(text("Save over").size(10))
                        .on_press(InterfaceMessage::SnapshotSaveStart(i))
                        .padding([3, 10])
                        .style(button::secondary),
                    button(if delete_pending { text("\u{26A0} again!").size(10) } else { text("Delete").size(10) })
                        .on_press(InterfaceMessage::DeleteSnapshot(i))
                        .padding([3, 10])
                        .style(if delete_pending { button::danger } else { button::secondary }),
                ]
                .spacing(4)
                .into()
            };

            row_items.push(
                column![btn, below]
                    .align_x(Center)
                    .spacing(4)
                    .width(Fill)
                    .into(),
            );
        }

        let remaining = GRID_COLS - row_items.len();
        for _ in 0..remaining {
            row_items.push(Space::new().width(Fill).into());
        }

        rows.push(
            row(row_items)
                .spacing(8)
                .width(Fill)
                .into(),
        );
    }

    let grid = column(rows).spacing(8);

    let reset_btn = button(
        text("Reset to Default").size(14).align_x(Center),
    )
    .on_press(InterfaceMessage::ResetMixer)
    .padding([12, 24])
    .width(Fill)
    .style(button::danger);

    container(
        column![
            text("Snapshots").size(14),
            Space::new().height(8),
            grid,
            Space::new().height(10),
            reset_btn,
        ]
        .align_x(Center)
        .spacing(4)
        .padding([12, 14])
        .width(Fill),
    )
    .style(container::rounded_box)
    .width(Fill)
    .into()
}
