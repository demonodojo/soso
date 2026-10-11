//! T29 — el resumen de campaña no maquilla el denominador ni inventa medidas.

use soso_improve_core::report::*;

fn ids(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("R{i:02}")).collect()
}

fn reg(id: &str, r: Cierre, fallo: Option<FalloDe>) -> Registro {
    Registro { id: id.into(), resultado: r, fallo, intentos: 1, herramientas: Some(5), tokens: Some(10_000), segundos: 600, inyeccion: None, nota: String::new() }
}

fn campana(aceptadas: usize) -> Vec<Registro> {
    ids(10)
        .iter()
        .enumerate()
        .map(|(i, id)| if i < aceptadas { reg(id, Cierre::Aceptada, None) } else { reg(id, Cierre::Rechazada, Some(FalloDe::Programacion)) })
        .collect()
}

fn con_inyeccion(mut r: Vec<Registro>, recuperada: bool) -> Vec<Registro> {
    r[1].inyeccion = Some(Inyeccion { que: "agente matado y coordinador reiniciado".into(), recuperada });
    r
}

#[test]
fn cinco_de_diez_con_recuperacion_y_base_verde_cumple_si4() {
    let r = resumir(&ids(10), &con_inyeccion(campana(5), true), Some(true)).unwrap();
    assert_eq!((r.total, r.aceptadas, r.rechazadas), (10, 5, 5));
    assert!(r.cumple_si4, "{:?}", r.motivos_no_cumple);
    assert_eq!(r.segundos_total, 6000);
    assert_eq!(r.segundos_por_aceptada, Some(1200), "se reparte todo lo gastado entre las aceptadas");
    assert_eq!(r.tokens_por_aceptada, Some(20_000));
    assert_eq!(r.intentos_por_aceptada, Some(2.0));
    assert_eq!(r.por_fallo[&FalloDe::Programacion], 5);
}

#[test]
fn cuatro_de_diez_no_cumple_aunque_todo_lo_demas_este_bien() {
    let r = resumir(&ids(10), &con_inyeccion(campana(4), true), Some(true)).unwrap();
    assert!(!r.cumple_si4);
    assert!(r.motivos_no_cumple.iter().any(|m| m.contains("4 aceptadas de 10")));
}

#[test]
fn sin_recuperacion_acreditada_o_sin_base_final_no_hay_go() {
    let sin_inyeccion = resumir(&ids(10), &campana(8), Some(true)).unwrap();
    assert!(!sin_inyeccion.cumple_si4 && sin_inyeccion.motivos_no_cumple.iter().any(|m| m.contains("recuperación no está acreditada")));
    let mala = resumir(&ids(10), &con_inyeccion(campana(8), false), Some(true)).unwrap();
    assert!(!mala.cumple_si4 && mala.motivos_no_cumple.iter().any(|m| m.contains("0 de 1 inyecciones")));
    let regresion = resumir(&ids(10), &con_inyeccion(campana(8), true), Some(false)).unwrap();
    assert!(regresion.motivos_no_cumple.iter().any(|m| m.contains("regresión")));
    let sin_comprobar = resumir(&ids(10), &con_inyeccion(campana(8), true), None).unwrap();
    assert!(sin_comprobar.motivos_no_cumple.iter().any(|m| m.contains("no se comprobó")));
}

#[test]
fn no_se_puede_quitar_ni_repetir_ni_anadir_tareas() {
    let mut r = campana(7);
    let quitada = r.remove(9);
    let e = resumir(&ids(10), &r, Some(true)).unwrap_err().to_string();
    assert!(e.contains("falta la tarea fijada R10") && e.contains("denominador"), "{e}");
    r.push(quitada.clone());
    r.push(quitada);
    assert!(resumir(&ids(10), &r, Some(true)).unwrap_err().to_string().contains("2 veces"));
    let mut extra = campana(7);
    extra.push(reg("R99", Cierre::Aceptada, None));
    assert!(resumir(&ids(10), &extra, Some(true)).unwrap_err().to_string().contains("no estaba entre las tareas fijadas"));
}

#[test]
fn las_medidas_que_faltan_no_se_rellenan() {
    let mut r = campana(6);
    r[3].tokens = None;
    let res = resumir(&ids(10), &r, Some(true)).unwrap();
    assert_eq!(res.tokens_por_aceptada, None, "con un hueco, el promedio es desconocido, no menor");
    assert!(res.sin_medir.iter().any(|m| m.contains("tokens: no se midió en R04")));
    assert_eq!(res.herramientas_por_aceptada, Some(8), "lo que sí se midió se calcula (50 herramientas entre 6 aceptadas)");
}

#[test]
fn sin_aceptadas_no_hay_promedios_y_los_fallos_se_clasifican() {
    let mut r = campana(0);
    r[0].fallo = Some(FalloDe::Infraestructura);
    r[1].fallo = Some(FalloDe::Protocolo);
    r[2].resultado = Cierre::Bloqueada;
    r[3].fallo = None;
    let res = resumir(&ids(10), &r, Some(true)).unwrap();
    assert_eq!((res.segundos_por_aceptada, res.tokens_por_aceptada, res.intentos_por_aceptada), (None, None, None));
    assert_eq!(res.bloqueadas, 1);
    assert_eq!(res.por_fallo[&FalloDe::Infraestructura], 1);
    assert_eq!(res.por_fallo[&FalloDe::Protocolo], 1);
    assert!(res.sin_medir.iter().any(|m| m.contains("R04: no se clasificó el fallo")));
    assert!(!res.cumple_si4);
}

#[test]
fn el_markdown_dice_go_o_no_go_y_lista_cada_tarea() {
    let r = con_inyeccion(campana(5), true);
    let res = resumir(&ids(10), &r, Some(true)).unwrap();
    let md = markdown(&res, &r);
    assert!(md.contains("5 aceptadas de 10") && md.contains("**GO**"));
    assert!(md.contains("| R02 | Aceptada |") && md.contains("recuperada"));
    let md2 = markdown(&resumir(&ids(10), &campana(2), Some(true)).unwrap(), &campana(2));
    assert!(md2.contains("**NO-GO**") && md2.contains("no cumple:"));
}
