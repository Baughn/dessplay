//! Offline graphics/layout smoke harness using the production terminal loop.
use dessplay::ui::{
    app::Ui,
    layout::{LayoutBundle, LayoutOptions},
    msg::UserAction,
    shell::{UiInput, run_input_thread, run_ui_thread},
};
use dessplay_core::types::UserId;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argument = std::env::args().nth(1);
    if argument
        .as_deref()
        .is_none_or(|arg| matches!(arg, "--help" | "-h"))
    {
        println!(
            "Usage: cargo run -p dessplay --example layout_smoke -- PATH\nExports missing layout defaults to PATH and opens an offline client.\nWheel/PgUp/PgDn: inspect gradient cropping; F3: form; F12: layout tools; Ctrl-C: quit.\nEdit PATH while running to exercise the production layout watcher.\nDESSPLAY_IMAGE_PROTOCOL=auto|kitty|sixel|iterm2|halfblocks selects graphics."
        );
        return Ok(());
    }
    let directory = std::path::PathBuf::from(argument.ok_or("missing layout path")?);
    LayoutBundle::init(&directory)?;
    let mut ui = Ui::with_setup(
        UserId::new("layout-smoke"),
        Default::default(),
        vec![],
        false,
    );
    ui.set_layout_options(LayoutOptions {
        directory: Some(directory),
        builtin: false,
    });
    let (inputs, receiver) = std::sync::mpsc::sync_channel(128);
    let (actions, mut action_receiver) = tokio::sync::mpsc::channel(128);
    let replies = inputs.clone();
    let worker = std::thread::spawn(move || {
        while let Some(action) = action_receiver.blocking_recv() {
            match action {
                UserAction::FetchChatImage { url } => {
                    let pixels = image::RgbaImage::from_fn(320, 960, |x, y| {
                        image::Rgba([
                            (y * 255 / 959) as u8,
                            (x * 255 / 319) as u8,
                            (255 - y * 255 / 959) as u8,
                            255,
                        ])
                    });
                    if replies
                        .send(UiInput::ChatImage {
                            url,
                            result: Ok(Box::new(image::DynamicImage::ImageRgba8(pixels))),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                UserAction::Quit => break,
                _ => {} // This harness has no persistence, player, or network actors.
            }
        }
    });
    run_ui_thread(ui, receiver, actions, move || {
        for index in 0..25 {
            let text = match index {
                18 => "https://layout.invalid/gradient.png".into(),
                19 => "Repeated URL (only its first occurrence has an image): https://layout.invalid/gradient.png".into(),
                24 => "Offline smoke: scroll the gradient; resize; open F3/F12; edit the layout files. 界 😀".into(),
                _ => format!("Context row {index}: wrapped text keeps its source position through layout reloads."),
            };
            if inputs
                .send(UiInput::Irc {
                    timestamp: 1_000 + index,
                    sender: "fixture".into(),
                    text,
                    action: false,
                })
                .is_err()
            {
                return;
            }
        }
        std::thread::spawn(move || run_input_thread(inputs));
    });
    worker.join().map_err(|_| "smoke action worker panicked")?;
    Ok(())
}
