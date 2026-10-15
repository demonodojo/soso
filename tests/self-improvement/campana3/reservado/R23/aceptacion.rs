//! Aceptación reservada de R23.

use soso_web_core::html::reflow_html;

fn lineas(html: &str, ancho: usize) -> Vec<String> {
    reflow_html(html, "https://a/", ancho)
        .text
        .lines()
        .map(|l| l.to_string())
        .filter(|l| !l.trim().is_empty())
        .collect()
}

/// Quita la viñeta que ya pone hoy `reflow_html` (su glifo concreto no importa) y comprueba
/// que es la misma en todos los elementos.
fn sin_vineta(l: Vec<String>) -> Vec<String> {
    let mut marca: Option<String> = None;
    l.into_iter()
        .map(|x| {
            let (m, resto) = x.split_once(' ').unwrap_or((x.as_str(), ""));
            assert!(!m.is_empty());
            match &marca {
                None => marca = Some(m.to_string()),
                Some(prev) => assert_eq!(prev, m, "viñetas distintas"),
            }
            resto.to_string()
        })
        .collect()
}

#[test]
fn li_sin_cerrar_empiezan_linea() {
    let l = sin_vineta(lineas("<ul><li>uno<li>dos<li>tres</ul>", 80));
    assert_eq!(l, vec!["uno", "dos", "tres"]);
}

#[test]
fn li_cerrados_siguen_igual() {
    let l = sin_vineta(lineas("<ul><li>uno</li><li>dos</li></ul>", 80));
    assert_eq!(l, vec!["uno", "dos"]);
    let out = reflow_html("<ul><li>uno</li><li>dos</li></ul>", "https://a/", 80).text;
    assert!(!out.contains("\n\n"), "{out:?}");
}

#[test]
fn li_con_enlace_sin_cerrar() {
    let out = reflow_html("<ul><li><a href=\"/x\">uno</a><li>dos</ul>", "https://a/", 80);
    let l: Vec<String> = out.text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.to_string()).collect();
    assert_eq!(sin_vineta(l), vec!["uno [1]", "dos"]);
    assert_eq!(out.links.len(), 1);
}

#[test]
fn hr_sin_barra_dibuja_regla() {
    let l = lineas("<p>antes</p><hr><p>despues</p>", 80);
    assert_eq!(l, vec!["antes".to_string(), "-".repeat(40), "despues".to_string()]);
}

#[test]
fn hr_con_barra_y_mayusculas() {
    let esperado = vec!["antes".to_string(), "-".repeat(40), "despues".to_string()];
    assert_eq!(lineas("<p>antes</p><hr/><p>despues</p>", 80), esperado);
    assert_eq!(lineas("<p>antes</p><HR><p>despues</p>", 80), esperado);
    assert_eq!(lineas("antes<hr>despues", 80), esperado);
}

#[test]
fn hr_respeta_el_ancho() {
    let l = lineas("a<hr>b", 25);
    assert_eq!(l, vec!["a".to_string(), "-".repeat(25), "b".to_string()]);
    // El mínimo de ancho es 20.
    let l = lineas("a<hr>b", 5);
    assert_eq!(l, vec!["a".to_string(), "-".repeat(20), "b".to_string()]);
}

#[test]
fn lo_demas_no_cambia() {
    let out = reflow_html("<p>Hola <a href=\"/x\">mundo</a>.</p><script>alert(1)</script>", "https://e.org/", 40);
    assert!(out.text.contains("[1]"));
    assert!(!out.text.contains("alert"));
    assert_eq!(out.links[0].href, "https://e.org/x");
}
