/* Código C ajeno, compilado desde fuente para el target de soso y enlazado
 * estáticamente en este programa. Es la mitad afirmativa del experimento
 * N-005: la pregunta no es si soso puede *cargar* código nativo en ejecución,
 * sino si el código nativo que un runtime necesita puede estar ya dentro del
 * binario.
 *
 * Lo que hace tiene que ser algo que **no** se pueda confundir con una
 * constante que el enlazador podría haber plegado: recorre memoria, usa el
 * montón del llamante y devuelve un resultado que depende de la entrada.
 * Si esto devolviera 42 siempre, el caso pasaría aunque la función no se
 * hubiera compilado nunca.
 */

/* Sin cabeceras: no hay libc que incluir. Los tipos se declaran a mano, que
 * es exactamente la fricción que N-006 tiene que medir en serio. */
typedef unsigned long size_t_;
typedef unsigned char u8_;

/* `memcpy` y `memset` los pone `compiler-builtins-mem`; son los únicos dos
 * símbolos de libc que el C de `ring` necesita, medido con `nm -u` sobre sus
 * 31 objetos. Aquí se usa uno para que el enlace lo demuestre. */
void *memcpy(void *dst, const void *src, size_t_ n);

/* Una suma con acarreo rotado: depende de cada byte y del orden. */
unsigned long soso_probe_mezcla(const u8_ *datos, size_t_ n) {
    unsigned long h = 1469598103934665603UL; /* FNV-1a de 64 bits */
    for (size_t_ i = 0; i < n; i++) {
        h ^= datos[i];
        h *= 1099511628211UL;
    }
    return h;
}

/* Y una que toca memoria del llamante, para que el enlace no sea sólo de
 * lectura: copia al revés usando el `memcpy` de compiler_builtins. */
void soso_probe_invertir(u8_ *dst, const u8_ *src, size_t_ n) {
    for (size_t_ i = 0; i < n; i++) {
        u8_ b = src[n - 1 - i];
        memcpy(dst + i, &b, 1);
    }
}
