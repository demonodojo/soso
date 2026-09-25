//! Sonda 11 — enterarse de que un fichero cambió:
//! [N-008](../../../../docs/self-improvement/native/N-008.md).
//!
//! La ficha del backlog dice «inotify o equivalente», y la tentación es
//! ponerse a escribir eventos en el kernel. Antes hay que contestar a lo que
//! [N-001](../../../../docs/self-improvement/native/N-001.md) enseñó a
//! preguntar: **¿qué cuesta la alternativa simple?** Allí la medida dio la
//! vuelta a la intuición de la ficha, y ahorró escribir lo que no había que
//! escribir.
//!
//! La alternativa simple aquí es **sondear `stat`**. Para que sirva hacen
//! falta dos cosas, y se miden por separado porque fallan por separado:
//!
//! - Que `mtime` **cambie** cuando el fichero cambia. Si no cambia, sondear no
//!   es «más lento»: es **incorrecto**, y ninguna optimización lo arregla.
//! - Que sondear un árbol **cueste poco**. Esto sí es una cuestión de número,
//!   y el número se mide, no se supone.
//!
//! El caso que de verdad decide es el segundo de la lista: dos escrituras
//! dentro del **mismo segundo**. `mtime` está en segundos de uptime, así que
//! ahí es donde un vigilante basado en sondeo se queda ciego — y un vigilante
//! que a veces no ve es peor que no tener vigilante, porque se confía en él.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libsoso::{abi, ciclos, println, sys};

use crate::{errno, limpiar_dir, unir, Caso};

const DIR: &str = "/var/self-improvement/probe/vigilancia";
/// Cuántos ficheros tiene el árbol que se sondea. Un proyecto de verdad tiene
/// más, pero la medida se extrapola linealmente y el coste por fichero es lo
/// que hace falta saber.
const CUANTOS: usize = 64;

fn anotar(casos: &mut Vec<Caso>, c: Caso) {
    if c.paso && c.esperado.is_empty() {
        println!("probe: {} medido: {}", c.id, c.observado);
    } else if c.paso {
        println!("probe: {} ok ({})", c.id, c.observado);
    } else {
        println!(
            "probe: {} FALLO esperado={:?} observado={:?}",
            c.id, c.esperado, c.observado
        );
    }
    casos.push(c);
}

fn escribir(ruta: &str, datos: &[u8]) -> Result<(), String> {
    let fd = sys::open(ruta, abi::O_WRONLY | abi::O_CREAT | abi::O_TRUNC);
    if fd < 0 {
        return Err(format!("open {ruta} = {}", errno(fd)));
    }
    let fd = fd as u64;
    let mut n = 0;
    while n < datos.len() {
        let w = sys::write(fd, &datos[n..]);
        if w <= 0 {
            sys::close(fd);
            return Err(format!("write = {}", errno(w)));
        }
        n += w as usize;
    }
    // Se cierra a propósito: con la caché por inodo de N-001 el contenido se
    // publica al cerrar el último descriptor, así que un vigilante no puede
    // ver un fichero **mientras** se escribe. Eso ya es un límite de cualquier
    // forma que tome esta ficha.
    sys::close(fd);
    Ok(())
}

fn estado(ruta: &str) -> Option<(u64, u64)> {
    let mut st = abi::Stat::default();
    if sys::stat(ruta, &mut st) != 0 {
        return None;
    }
    Some((st.mtime, st.size))
}

pub fn ejecutar() -> Vec<Caso> {
    let mut casos: Vec<Caso> = Vec::new();
    sys::mkdir("/var/self-improvement");
    sys::mkdir("/var/self-improvement/probe");
    limpiar_dir(DIR);

    let uno = unir(DIR, "uno.txt");
    if let Err(e) = escribir(&uno, b"primera\n") {
        anotar(&mut casos, Caso::nuevo("vigilancia/preparar", "un fichero", e));
        return casos;
    }
    let Some((mtime0, size0)) = estado(&uno) else {
        anotar(&mut casos, Caso::nuevo("vigilancia/preparar", "stat", String::from("stat falló")));
        return casos;
    };

    // 1. **Cambiar el tamaño se ve.** Es el caso barato y el que hace que los
    //    dos siguientes signifiquen algo: si ni esto se viera, sondear estaría
    //    descartado de entrada.
    if let Err(e) = escribir(&uno, b"segunda escritura, mas larga\n") {
        anotar(&mut casos, Caso::nuevo("vigilancia/escribir", "ok", e));
        return casos;
    }
    let (mtime1, size1) = estado(&uno).unwrap_or((0, 0));
    // El veredicto va en la **misma forma** que lo esperado. Escribir aquí
    // «tamaño distinto de 8» frente a «tamaño 29» haría fallar el caso con el
    // resultado correcto delante: la comprobación compara cadenas, no lee.
    anotar(
        &mut casos,
        Caso::nuevo(
            "vigilancia/un-cambio-de-tamano-se-nota",
            "cambió",
            {
                // El detalle va al `println!`, **no** al veredicto: si el
                // observado lleva los números y el esperado no, el caso falla
                // con el resultado correcto delante. Me pasó dos veces
                // seguidas en esta misma sonda.
                println!("vigilancia: el tamaño pasó de {size0} a {size1}");
                if size1 != size0 { "cambió" } else { "no cambió" }
            },
        ),
    );

    // 2. **El caso que decide**: dos escrituras del **mismo tamaño**, seguidas.
    //    `mtime` está en segundos, así que si además caen en el mismo segundo,
    //    el fichero cambió y `stat` dice exactamente lo mismo que antes.
    //    No juzga: **informa**. Poner aquí un veredicto sería decidir la forma
    //    de la ficha antes de mirar el dato, que es justo lo que N-001 enseñó
    //    a no hacer.
    //
    // **Del mismo tamaño de verdad**: en el primer intento escribí 30 bytes
    // donde había 29 y el caso «midió» una diferencia de tamaño creyendo que
    // medía la ceguera de `mtime`. Una medida que no comprueba su propia
    // premisa mide otra cosa.
    const IGUAL_A: &[u8] = b"segunda escritura, mas larga\n";
    const IGUAL_B: &[u8] = b"TERCERA escritura, mas larga\n";
    //
    // **Y se repite**, porque el punto ciego es intermitente: depende de si
    // las dos escrituras caen en el mismo segundo. Una sola pasada dice
    // «INDISTINGUIBLES» o «se distinguen» según la suerte, y las dos cosas se
    // han visto. Una tasa es un dato; una anécdota no — y un punto ciego
    // intermitente es **peor** que uno constante, porque pasa las pruebas casi
    // siempre y falla cuando nadie mira.
    const PARES: usize = 6;
    let mut ciegos = 0usize;
    let mut t_escritura = 0u64;
    // Qué se escribió el último, para que el caso siguiente compare contra el
    // valor y no contra un literal. Escribir «TERC» a mano en la expectativa
    // ya me ha fallado aquí: en cuanto el bucle alternó, la comprobación
    // seguía mirando lo de antes. Una expectativa que duplica un dato que vive
    // en el código deja de ser cierta sin avisar.
    let mut ultimo: &[u8] = IGUAL_B;
    let (mut antes, mut despues) = ((0u64, 0u64), (0u64, 0u64));
    for i in 0..PARES {
        let (a, b) = if i % 2 == 0 { (IGUAL_A, IGUAL_B) } else { (IGUAL_B, IGUAL_A) };
        let _ = escribir(&uno, a);
        let x = estado(&uno).unwrap_or((0, 0));
        let t0 = ciclos();
        let _ = escribir(&uno, b);
        t_escritura = ciclos() - t0;
        let y = estado(&uno).unwrap_or((0, 0));
        if x == y {
            ciegos += 1;
        }
        antes = x;
        despues = y;
        ultimo = b;
    }
    println!("vigilancia: {ciegos} de {PARES} pares de escrituras fueron indistinguibles por stat");
    anotar(
        &mut casos,
        Caso::nuevo(
            "vigilancia/las-dos-escrituras-eran-del-mismo-tamano",
            "iguales",
            if IGUAL_A.len() == IGUAL_B.len() {
                String::from("iguales")
            } else {
                format!("{} y {}", IGUAL_A.len(), IGUAL_B.len())
            },
        ),
    );
    println!("vigilancia: una escritura completa costó {t_escritura} ciclos");
    let ciego = ciegos > 0;
    anotar(
        &mut casos,
        Caso::observacion(
            "vigilancia/dos-escrituras-del-mismo-tamano",
            format!(
                "{ciegos} de {PARES} pares indistinguibles; el último: antes (mtime {}, tamaño {}), después (mtime {}, tamaño {}) → {}",
                antes.0,
                antes.1,
                despues.0,
                despues.1,
                if ciego {
                    "hubo pares INDISTINGUIBLES"
                } else {
                    "todos se distinguieron"
                }
            ),
        ),
    );

    // 3. Y el contenido **sí** cambió: sin esto, «indistinguibles» podría
    //    significar que la escritura no llegó a ocurrir, que es otra cosa.
    let fd = sys::open(&uno, abi::O_RDONLY);
    let mut buf = [0u8; 8];
    let leidos = if fd >= 0 {
        let n = sys::read(fd as u64, &mut buf);
        sys::close(fd as u64);
        n.max(0) as usize
    } else {
        0
    };
    anotar(
        &mut casos,
        Caso::nuevo(
            "vigilancia/el-contenido-si-cambio",
            format!("empieza como lo último escrito ({:?})", &ultimo[..4]),
            if leidos >= 4 && buf[..4] == ultimo[..4] {
                format!("empieza como lo último escrito ({:?})", &ultimo[..4])
            } else {
                format!("{:?}", &buf[..leidos])
            },
        ),
    );
    let _ = mtime0;
    let _ = mtime1;

    // 4. **Cuánto cuesta sondear.** Con `rdtsc`, no con el reloj de
    //    milisegundos: el PIT subcuenta durante el polling de disco, y esto es
    //    justo un camino de E/S.
    let mut rutas = Vec::with_capacity(CUANTOS);
    for i in 0..CUANTOS {
        let r = unir(DIR, &format!("f{i:03}.txt"));
        let _ = escribir(&r, b"x\n");
        rutas.push(r);
    }
    // Se mide **dos veces**: la primera pasada puede estar pagando el primer
    // acceso a cada inodo, y una cifra que sólo se toma una vez no distingue
    // «stat es caro» de «la primera vez es cara». La segunda es la que dice
    // cuánto cuesta sondear *repetidamente*, que es lo que hace un vigilante.
    let mut medir = || {
        let t0 = ciclos();
        let mut vistos = 0usize;
        for r in &rutas {
            if estado(r).is_some() {
                vistos += 1;
            }
        }
        (ciclos() - t0, vistos)
    };
    let (frio, vistos) = medir();
    let (caliente, _) = medir();
    let n = rutas.len().max(1) as u64;
    println!(
        "vigilancia: sondear {vistos} ficheros costó {frio} ciclos en frío ({} por fichero) y {caliente} en caliente ({} por fichero)",
        frio / n,
        caliente / n
    );
    anotar(
        &mut casos,
        Caso::observacion(
            "vigilancia/coste-de-sondear",
            format!(
                "{vistos} ficheros: frío {frio} ({} por fichero), caliente {caliente} ({} por fichero)",
                frio / n,
                caliente / n
            ),
        ),
    );

    // 5. **El control que faltaba: ¿cuánto cuesta un syscall cualquiera?**
    //    Decir «`stat` es caro» sin esto es dar por hecho que el coste está en
    //    el sistema de ficheros. Si un `getpid` costara lo mismo, la
    //    conclusión sería otra y mucho más grande: que lo caro es **entrar al
    //    kernel**, y entonces mirar dentro de sosofs sería mirar donde no es.
    {
        const VUELTAS: u64 = 64;
        let t0 = ciclos();
        let mut acc = 0u64;
        for _ in 0..VUELTAS {
            acc = acc.wrapping_add(sys::getpid());
        }
        let c = (ciclos() - t0) / VUELTAS;
        println!("vigilancia: un getpid cuesta {c} ciclos (pid {acc:x} para que no lo borre el optimizador)");
        anotar(
            &mut casos,
            Caso::observacion("vigilancia/coste-de-un-syscall-trivial", format!("{c} ciclos")),
        );
    }

    let hondo_ruta = unir(DIR, "f000.txt");

    // 5b. **¿Qué queda en el suelo de un `stat`?** Con la ruta ya resuelta de
    //     una sola reserva, quedan tres cosas: entrar al kernel, reservar la
    //     cadena, y recorrer el árbol. Separarlas se puede sin instrumentar el
    //     kernel: una ruta **más larga que PATH_MAX** se rechaza *antes* de
    //     reservar y antes de tocar el árbol, así que mide el syscall pelado
    //     con su copia de la ruta desde espacio de usuario.
    {
        let mut larga = String::from("/");
        while larga.len() < 400 {
            larga.push_str("xxxxxxxx/");
        }
        const VUELTAS: u64 = 32;
        let mut min = u64::MAX;
        for _ in 0..VUELTAS {
            let t0 = ciclos();
            let mut st = abi::Stat::default();
            let _ = sys::stat(&larga, &mut st);
            let c = ciclos() - t0;
            if c < min {
                min = c;
            }
        }
        println!("vigilancia: un stat rechazado por largo (sin reserva ni árbol) = {min} ciclos");
        anotar(
            &mut casos,
            Caso::observacion("vigilancia/suelo-del-syscall-con-ruta", format!("{min} ciclos")),
        );
    }

    // 6. **¿Va al disco o no?** El instrumento ya existía: `SYS_IOSTAT` cuenta
    //    peticiones al dispositivo. Leerlo antes y después de una tanda de
    //    `stat` dice cuántas provoca cada uno — que es la pregunta que separa
    //    «la caché no sirve» de «la caché sirve y el tiempo se va en recorrer
    //    el árbol». N-012 se quedó ahí por adivinar desde el host en vez de
    //    mirar aquí.
    {
        const VUELTAS: u64 = 16;
        let mut a = abi::IoStat::default();
        let mut b = abi::IoStat::default();
        sys::iostat(&mut a);
        let t0 = ciclos();
        for _ in 0..VUELTAS {
            let _ = estado(&hondo_ruta);
        }
        let c = (ciclos() - t0) / VUELTAS;
        sys::iostat(&mut b);
        let peticiones = b.peticiones.saturating_sub(a.peticiones);
        let bloques = b.bloques.saturating_sub(a.bloques);
        let nanos = b.nanos.saturating_sub(a.nanos);
        println!(
            "vigilancia: {VUELTAS} stat profundos = {peticiones} peticiones al disco, {bloques} bloques, {nanos} ns en el dispositivo; {c} ciclos por stat"
        );
        anotar(
            &mut casos,
            Caso::observacion(
                "vigilancia/peticiones-al-disco-por-stat",
                format!(
                    "{} peticiones y {} bloques por stat, {} ns en el dispositivo por stat",
                    peticiones / VUELTAS,
                    bloques / VUELTAS,
                    nanos / VUELTAS
                ),
            ),
        );
    }

    // 7. **¿De dónde sale ese coste?** En caliente cuesta lo mismo que en
    //    frío, así que no es la primera lectura. La siguiente sospecha es la
    //    ruta: `stat` resuelve componente a componente, y cada componente es
    //    un `lookup` sobre el árbol. Si el coste sube con la profundidad, eso
    //    lo confirma y da un sitio donde mirar; si no sube, la sospecha estaba
    //    mal y hay que buscar en otro lado.
    for (etiqueta, ruta) in [("/", "/"), ("/var", "/var"), ("hondo", hondo_ruta.as_str())] {
        // Componentes de verdad, no barras: `/` tiene una barra y **cero**
        // componentes, y etiquetar la medida mal es como no tomarla.
        let profundidad = ruta.split('/').filter(|c| !c.is_empty()).count();
        // **Mínimo y media, no sólo media.** Aquí no hay un reloj de proceso:
        // `rdtsc` cuenta tiempo de pared, así que si el planificador desaloja
        // a este proceso en mitad de una vuelta, esa vuelta se lleva todo el
        // rato del otro. La media lo reparte y lo disfraza de coste; el mínimo
        // es la única vuelta de la que se puede decir que midió el trabajo y
        // nada más.
        const VUELTAS: usize = 32;
        let mut min = u64::MAX;
        let mut suma = 0u64;
        for _ in 0..VUELTAS {
            let t0 = ciclos();
            let _ = estado(ruta);
            let c = ciclos() - t0;
            suma += c;
            if c < min {
                min = c;
            }
        }
        let media = suma / VUELTAS as u64;
        println!(
            "vigilancia: stat de {etiqueta} ({profundidad} componentes) = {min} ciclos mínimo, {media} de media"
        );
        anotar(
            &mut casos,
            Caso::observacion(
                &format!("vigilancia/coste-por-profundidad-{etiqueta}"),
                format!("{profundidad} componentes, {min} ciclos mínimo, {media} media"),
            ),
        );
    }

    casos
}
