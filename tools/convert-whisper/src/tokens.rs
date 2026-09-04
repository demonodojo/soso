//! Tokens especiales Whisper (multilingüe, n_vocab=51865).

const LANGS: [&str; 99] = [
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it",
    "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur",
    "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si",
    "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha",
    "ba", "jw", "su",
];

/// Rellena `pieces[50257..n_vocab)` con los tokens especiales de Whisper.
pub fn pad_whisper_special_tokens(pieces: &mut Vec<String>, n_vocab: usize) {
    pieces.resize(n_vocab, String::new());
    if n_vocab <= 50257 {
        return;
    }
    pieces[50257] = "<|endoftext|>".into();
    pieces[50258] = "<|startoftranscript|>".into();
    for (i, lang) in LANGS.iter().enumerate() {
        let id = 50259 + i;
        if id >= n_vocab {
            break;
        }
        pieces[id] = format!("<|{lang}|>");
    }
    if n_vocab > 50358 {
        pieces[50358] = "<|translate|>".into();
    }
    if n_vocab > TRANSCRIBE as usize {
        pieces[TRANSCRIBE as usize] = "<|transcribe|>".into();
    }
    if n_vocab > 50360 {
        pieces[50360] = "<|startoflm|>".into();
    }
    if n_vocab > 50361 {
        pieces[50361] = "<|startofprev|>".into();
    }
    if n_vocab > 50362 {
        pieces[50362] = "<|nospeech|>".into();
    }
    if n_vocab > NO_TIMESTAMPS as usize {
        pieces[NO_TIMESTAMPS as usize] = "<|notimestamps|>".into();
    }
    for i in 0..1501usize {
        let id = 50364 + i;
        if id >= n_vocab {
            break;
        }
        pieces[id] = format!("<|{:.2}|>", i as f64 * 0.02);
    }
}

/// Índice de idioma Whisper (p. ej. `es` → 3) o el token si `idioma` ya es id crudo.
#[cfg(test)]
pub fn resolve_lang_token(idioma: u32) -> u32 {
    if (50259..=50357).contains(&idioma) {
        return idioma;
    }
    if idioma < LANGS.len() as u32 {
        return 50259 + idioma;
    }
    idioma
}

pub const SOT: u32 = 50258;
pub const TRANSCRIBE: u32 = 50359;
pub const NO_TIMESTAMPS: u32 = 50363;
pub const EOT: u32 = 50257;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanish_lang_token() {
        assert_eq!(resolve_lang_token(3), 50262);
        assert_eq!(resolve_lang_token(50262), 50262);
    }
}
