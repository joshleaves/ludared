use super::project::ProjectFixture;

/// The real SNES LoROM image shipped in `fixtures/data`.
///
/// 64 KiB of test data, which the LoROM codec splits into exactly two 32 KiB
/// banks. It is binary test data rather than source, so the package excludes it.
pub const SOURCE: &[u8] = include_bytes!("data/smashing_the_stack.sfc");

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
const EXTRACT: &str = "std/generic/extract_bytes";

/// A project whose single source is the real LoROM fixture, carrying a decode
/// tree the real codecs can replay.
///
/// The tree is the one this project's tests care about: the LoROM codec splits
/// the source into banks, and a nested `extract_bytes` decode pulls the article
/// title out of the first of them. A test that unpacks the project therefore
/// ends up with the virtual paths a real source produces, nested artifact
/// included, without having to spell the tree out.
///
/// The source name is randomised so that each test's virtual paths stay to
/// itself, and the fixture owns its cache, so nothing leaks between tests.
///
/// Nothing is unpacked here: a test is free to leave the cache empty and build it
/// itself, or to hand it to [`Project::unpack`]. The fixture is returned alongside
/// its source name, since the virtual paths of every artifact depend on it.
///
/// [`Project::unpack`]: crate::project::Project::unpack
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
    "{source_name}": {{ "sha256": "unverified", "size": null, "label": null }}
  }},
  "decodes": {{
    "{source_name}": [ {title} ]
  }}
}}"#,
    title = lorom_decode(),
  )
}

/// The decode node splitting the source into banks, with the title nested under
/// the first bank.
fn lorom_decode() -> String {
  format!(
    r#"{{
      "name": "rom_banks",
      "codec": {{ "id": "{LOROM}", "version": 1, "args": {{}} }},
      "outputs": [ "{FIRST_BANK}", "{SECOND_BANK}" ],
      "decodes": {{
        "{FIRST_BANK}": [ {{
          "name": "TITLE",
          "codec": {{
            "id": "{EXTRACT}",
            "version": 1,
            "args": {{ "target": "{TITLE_FILE}", "offset": {TITLE_OFFSET}, "length": {TITLE_LENGTH} }}
          }},
          "outputs": [ "{TITLE_FILE}" ],
          "decodes": {{}}
        }}]
      }}
    }}"#
  )
}
