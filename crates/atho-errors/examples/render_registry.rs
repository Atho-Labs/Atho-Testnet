// SPDX-License-Identifier: Apache-2.0
// Copyright (c) Atho contributors

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("markdown"));
    match mode.as_str() {
        "markdown" => print!("{}", atho_errors::render_markdown_registry()),
        "json" => {
            print!(
                "{}",
                atho_errors::render_json_registry().expect("render json registry")
            )
        }
        other => panic!("unsupported registry output mode: {other}"),
    }
}
