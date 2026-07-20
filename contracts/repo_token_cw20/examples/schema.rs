use std::fs::{create_dir_all, write};
use std::path::PathBuf;

use cosmwasm_schema::{generate_api, remove_schemas};
use repo_token_cw20::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};

fn main() {
    let mut out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    out_dir.push("schema");
    create_dir_all(&out_dir).unwrap();
    remove_schemas(&out_dir).unwrap();

    let api = generate_api! {
        instantiate: InstantiateMsg,
        execute: ExecuteMsg,
        query: QueryMsg,
    }
    .render();

    let path = out_dir.join(concat!(env!("CARGO_PKG_NAME"), ".json"));
    write(&path, api.to_string().unwrap() + "\n").unwrap();
    println!("Exported the full API as {}", path.to_str().unwrap());

    let raw_dir = out_dir.join("raw");
    create_dir_all(&raw_dir).unwrap();

    for (filename, json) in api.to_schema_files().unwrap() {
        let path = raw_dir.join(filename);
        write(&path, json + "\n").unwrap();
        println!("Exported {}", path.to_str().unwrap());
    }
}
