use super::project::ProjectFixture;

/// The real SNES LoROM image shipped in `fixtures/data`.
///
/// 64 KiB of test data, which the LoROM codec splits into exactly two 32 KiB
/// banks. It is binary test data rather than source, so the package excludes it.
pub const SOURCE: &[u8] = include_bytes!("snes/smashing_the_stack.sfc");

/// The bytes of the article title the nested decode pulls out of the first bank.
pub const TITLE: &[u8] = b"Smashing The Stack For Fun And Profit";

/// The first bank the LoROM codec produces, which the nested decode hangs off.
const FIRST_BANK: &str = "rom_bank_00.bin";

/// The second bank the LoROM codec produces, recorded so that the replayed tree
/// is checked against every output it is supposed to produce.
const SECOND_BANK: &str = "rom_bank_01.bin";

/// The artifact nested in the first bank, relative to the source.
pub const TITLE_ARTIFACT: &str = "rom_bank_00.bin/TITLE.txt";

/// The file name the nested decode gives that artifact.
const TITLE_FILE: &str = "TITLE.txt";

/// Where the article title sits inside the first bank.
const TITLE_OFFSET: usize = 385;

/// How much of the first bank the article title takes up.
const TITLE_LENGTH: usize = 37;

const LOROM: &str = "std/nintendo/snes/cart/lorom";
const EXTRACT: &str = "std/generic/extract";

/// Creates an isolated project with a LoROM decode and a nested title extraction.
pub fn project() -> (ProjectFixture, String) {
  let (source_name, source_path) = ProjectFixture::random_source_name();
  let mut fixture = ProjectFixture::new();

  fixture.register_source_file(&source_path, SOURCE);
  fixture.write_manifest(manifest(&source_name));
  fixture.reload();

  (fixture, source_name)
}

/// The virtual path of the article title nested in the source's first bank.
pub fn title_vpath(source_name: &str) -> String {
  format!("{source_name}/{TITLE_ARTIFACT}")
}

/// A manifest recording the LoROM source and the decode tree replayed from it.
fn manifest(source_name: &str) -> String {
  format!(
    r#"{{
  "sources": {{
    "{source_name}": {{ "sha256": "{}", "size": null, "label": null }}
  }},
  "decodes": {{
    "{source_name}": [ {title} ]
  }}
}}"#,
    ProjectFixture::sha256_hex(SOURCE),
    title = lorom_decode(),
  )
}

/// The decode node splitting the source into banks, with the title nested under
/// the first bank.
fn lorom_decode() -> String {
  // The manifest records arguments as the string the codec is handed, so the JSON
  // is written there as a JSON string rather than as a nested object.
  let title_args = serde_json::json!({
    "name": TITLE_FILE,
    "offset": TITLE_OFFSET,
    "length": TITLE_LENGTH
  })
  .to_string();
  let title_args = serde_json::Value::String(title_args).to_string();

  format!(
    r#"{{
      "name": "rom_banks",
      "codec": {{ "id": "{LOROM}", "version": 1, "args": null }},
      "outputs": [ "{FIRST_BANK}", "{SECOND_BANK}" ],
      "decodes": {{
        "{FIRST_BANK}": [ {{
          "name": "TITLE",
          "codec": {{
            "id": "{EXTRACT}",
            "version": 1,
            "args": {title_args}
          }},
          "outputs": [ "{TITLE_FILE}" ],
          "decodes": {{}}
        }}]
      }}
    }}"#
  )
}
