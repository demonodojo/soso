//! Target incorporado. El guest no puede buscar el JSON: `Path::is_file` no está.

use crate::spec::Target;

pub(crate) fn target() -> Target {
    // C-035: el host soso no busca el JSON en disco.
    let (target, _warnings) = Target::from_json(include_str!(
        "../../../../../x86_64-unknown-soso.json"
    ))
    .expect("especificacion de x86_64-unknown-soso");
    target
}
