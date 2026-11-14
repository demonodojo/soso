// G1: backend GPU de la escena de primitivas del escritorio (GPRIM / GSCEN v2).
// FUENTE ÚNICA: la compila NVRTC (tools/gpu-prim/cubin.py) para el SASS del kernel de soso
// y tools/gpu-prim/validar.py para validarlo en el host.
// Un hilo por píxel recorre la lista en orden, así que el resultado es idéntico al backend
// CPU de user/sal/escritorio/escena.sal (esc_cpu_ejecutar). Formato de primitiva: 16 palabras
// de 32 bits; ver crates/soso-wm-proto/src/gprim.rs. Sin cabeceras de CUDA (NVRTC).

#define K_SOLIDO 1
#define K_DEGRADADO 2
#define K_RRECT 3
#define K_ANILLO 4
#define K_REJILLA 5
#define K_SOMBRA 6

__device__ __forceinline__ int imin(int a, int b) { return a < b ? a : b; }
__device__ __forceinline__ int imax(int a, int b) { return a > b ? a : b; }

__device__ __forceinline__ unsigned mezclar(unsigned fg, unsigned bg, int alfa) {
    int inv = 255 - alfa;
    unsigned r = (((fg >> 16) & 255) * alfa + ((bg >> 16) & 255) * inv) / 255;
    unsigned g = (((fg >> 8) & 255) * alfa + ((bg >> 8) & 255) * inv) / 255;
    unsigned b = ((fg & 255) * alfa + (bg & 255) * inv) / 255;
    return (r << 16) | (g << 8) | b;
}

__device__ __forceinline__ unsigned a_palabra(unsigned rgb, int fmt) {
    if (fmt == 1) return 0xFF000000u | rgb;
    return 0xFF000000u | ((rgb & 255) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 255);
}

__device__ __forceinline__ unsigned de_palabra(unsigned w, int fmt) {
    if (fmt == 1) return w & 0xFFFFFF;
    return ((w & 255) << 16) | (w & 0xFF00) | ((w >> 16) & 255);
}

// Aplica `rgb` con `alfa` sobre el píxel (misma regla que esc_cpu_relleno).
__device__ __forceinline__ void poner(unsigned* px, unsigned rgb, int alfa, int fmt) {
    if (alfa >= 255) { *px = a_palabra(rgb, fmt); }
    else if (alfa > 0) { *px = a_palabra(mezclar(rgb, de_palabra(*px, fmt), alfa), fmt); }
}

__device__ __forceinline__ int iclamp(int v, int lo, int hi) { return v < lo ? lo : (v > hi ? hi : v); }

// Las funciones de cobertura copian las de user/sal/escritorio/superficie.sal (rr_isqrt,
// rr_esquina_cov, rr_cov16) y escena.sal (esc_dist_rrect4, esc_sombra_alfa) operación a
// operación, en enteros: el resultado tiene que ser el mismo píxel a píxel.
__device__ __forceinline__ long long rr_isqrt(long long v) {
    if (v <= 0) return 0;
    long long x = v, y = (x + 1) / 2;
    while (y < x) { x = y; y = (x + v / x) / 2; }
    return x;
}

__device__ __forceinline__ int rr_esquina_cov(int dx16, int dy16, int r) {
    long long d = rr_isqrt((long long)dx16 * dx16 + (long long)dy16 * dy16);
    return iclamp((int)(r * 16 + 8 - d), 0, 16);
}

// Cobertura 0..16 de (x,y) por el rectángulo redondeado (radios ya recortados).
__device__ __forceinline__ int rr_cov16(int x, int y, int x0, int y0, int x1, int y1,
                                        int a, int b, int c, int d) {
    if (x < x0 || x >= x1 || y < y0 || y >= y1) return 0;
    int px = x * 16 + 8, py = y * 16 + 8;
    if (x - x0 < a && y - y0 < a) return rr_esquina_cov((x0 + a) * 16 - px, (y0 + a) * 16 - py, a);
    if (x1 - 1 - x < b && y - y0 < b) return rr_esquina_cov(px - (x1 - b) * 16, (y0 + b) * 16 - py, b);
    if (x1 - 1 - x < c && y1 - 1 - y < c) return rr_esquina_cov(px - (x1 - c) * 16, py - (y1 - c) * 16, c);
    if (x - x0 < d && y1 - 1 - y < d) return rr_esquina_cov((x0 + d) * 16 - px, py - (y1 - d) * 16, d);
    return 16;
}

// Igual, recortando antes los radios al rectángulo (rr_radio_ok).
__device__ __forceinline__ int rrect_cov16(int x, int y, int x0, int y0, int x1, int y1,
                                           int rtl, int rtr, int rbr, int rbl) {
    int w = x1 - x0, h = y1 - y0;
    if (w <= 0 || h <= 0) return 0;
    int m = imin(w / 2, h / 2);
    return rr_cov16(x, y, x0, y0, x1, y1, imin(rtl, m), imin(rtr, m), imin(rbr, m), imin(rbl, m));
}

__device__ __forceinline__ int dist_rrect4(int px, int py, int x0, int y0, int x1, int y1, int r) {
    int p = px * 4 + 2, q = py * 4 + 2, r4 = r * 4;
    int ax = x0 * 4 + r4, bx = x1 * 4 - r4, ay = y0 * 4 + r4, by = y1 * 4 - r4;
    int dx = imax(imax(ax - p, p - bx), 0);
    int dy = imax(imax(ay - q, q - by), 0);
    if (dx > 0 && dy > 0) return (int)rr_isqrt((long long)dx * dx + (long long)dy * dy) - r4;
    if (dx > 0) return dx - r4;
    if (dy > 0) return dy - r4;
    return 0 - imin(imin(p - x0 * 4, x1 * 4 - p), imin(q - y0 * 4, y1 * 4 - q));
}

__device__ __forceinline__ int sombra_alfa(int d4, int ext, int alfa) {
    int dentro = ext * 4 / 3;
    long long span = ext * 4 + dentro;
    long long u = span - (d4 + dentro);
    if (u <= 0) return 0;
    if (u >= span) return alfa;
    return (int)((long long)alfa * u * u * (3 * span - 2 * u) / (span * span * span));
}

// Bloques de una dimensión (el lanzador de soso fija ntid = (256,1,1)): x = columna, y = fila.
extern "C" __global__ void gprim_rt(unsigned* rt, int pitch_px, int ancho, int alto, int fmt,
                                    const int* prims, int n,
                                    const unsigned* celdas, const unsigned char* psf) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y;
    if (x >= ancho || y >= alto) return;
    unsigned* px = rt + (size_t)y * pitch_px + x;
    for (int i = 0; i < n; i++) {
        const int* p = prims + i * 16;
        int kind = p[0];
        int x0 = p[1], y0 = p[2], x1 = p[3], y1 = p[4];
        unsigned c0 = (unsigned)p[5] & 0xFFFFFF, c1 = (unsigned)p[6] & 0xFFFFFF;
        int alfa = p[7];
        if (kind == K_SOLIDO) {
            if (x >= imax(0, x0) && x < imin(ancho, x1) && y >= imax(0, y0) && y < imin(alto, y1))
                poner(px, c0, alfa, fmt);
        } else if (kind == K_DEGRADADO) {
            int ya = imax(0, y0), yb = imin(alto, y1), xa = imax(0, x0), xb = imin(ancho, x1);
            int hh = yb - ya;
            if (hh > 0 && x >= xa && x < xb && y >= ya && y < yb) {
                int t = y - ya, den = hh - 1;
                unsigned c;
                if (den <= 0) c = c0;
                else {
                    int inv = den - t;
                    unsigned r = ((((c0 >> 16) & 255) * inv) + (((c1 >> 16) & 255) * t)) / den;
                    unsigned g = ((((c0 >> 8) & 255) * inv) + (((c1 >> 8) & 255) * t)) / den;
                    unsigned b = (((c0 & 255) * inv) + ((c1 & 255) * t)) / den;
                    c = (r << 16) | (g << 8) | b;
                }
                *px = a_palabra(c, fmt);
            }
        } else if (kind == K_RRECT) {
            // Bordes de esquina suavizados: la cobertura escala el alfa (rellenar_rrect4).
            unsigned rd = (unsigned)p[8];
            int rtl = rd & 255, rtr = (rd >> 8) & 255, rbr = (rd >> 16) & 255, rbl = (rd >> 24) & 255;
            int bw = p[10];
            if (x < 0 || x >= ancho || y < 0 || y >= alto) continue;
            if (bw > 0) {
                int cb = rrect_cov16(x, y, x0, y0, x1, y1, rtl, rtr, rbr, rbl);
                poner(px, (unsigned)p[9] & 0xFFFFFF, 255 * cb / 16, fmt);
                int ci = rrect_cov16(x, y, x0 + bw, y0 + bw, x1 - bw, y1 - bw,
                                     imax(0, rtl - bw), imax(0, rtr - bw), imax(0, rbr - bw), imax(0, rbl - bw));
                poner(px, c0, alfa * ci / 16, fmt);
            } else {
                int cv = rrect_cov16(x, y, x0, y0, x1, y1, rtl, rtr, rbr, rbl);
                poner(px, c0, alfa * cv / 16, fmt);
            }
        } else if (kind == K_SOMBRA) {
            // Sombra difusa de una ventana redondeada; no toca lo que la ventana cubre del todo.
            int wx0 = p[11], wy0 = p[12], wx1 = p[13], wy1 = p[14];
            int ext = p[10], dy = p[15];
            if (ext <= 0) continue;
            if (x < imax(0, x0) || x >= imin(ancho, x1) || y < imax(0, y0) || y >= imin(alto, y1)) continue;
            int rr = (int)((unsigned)p[8] & 255);
            int r = imin(rr, imin((wx1 - wx0) / 2, (wy1 - wy0) / 2));
            if (rr_cov16(x, y, wx0, wy0, wx1, wy1, r, r, r, r) >= 16) continue;
            int a = sombra_alfa(dist_rrect4(x, y, wx0, wy0 + dy, wx1, wy1 + dy, r), ext, alfa);
            if (a > 0) poner(px, c0, a, fmt);
        } else if (kind == K_REJILLA) {
            // Rejilla de celdas de texto: fondo opaco por celda y tinta donde el glifo tiene bit.
            int cols = p[11], filas = p[12], gw = p[13], gh = p[14], bpg = p[8], hdr = p[9], c0i = p[15];
            int rx = x - x0, ry = y - y0;
            if (rx >= 0 && ry >= 0 && gw > 0 && gh > 0 && rx < cols * gw && ry < filas * gh) {
                int cx = rx / gw, col = rx % gw, cy = ry / gh, row = ry % gh;
                const unsigned* c = celdas + (size_t)(c0i + cy * cols + cx) * 3;
                *px = a_palabra(c[2] & 0xFFFFFF, fmt);
                if (c[0] != 0xFFFFFFFFu) {
                    if (psf[12] & 2) {
                        // Glifos de cobertura de 8 bits (sosomono-*.psf2): mezcla tinta y fondo.
                        unsigned v = psf[hdr + c[0] * bpg + row * gw + col];
                        if (v >= 255) *px = a_palabra(c[1] & 0xFFFFFF, fmt);
                        else if (v > 0) *px = a_palabra(mezclar(c[1] & 0xFFFFFF, c[2] & 0xFFFFFF, (int)v), fmt);
                    } else {
                        int bpr = (gw + 7) / 8;
                        unsigned v = psf[hdr + c[0] * bpg + row * bpr + col / 8];
                        if ((v >> (7 - (col & 7))) & 1) *px = a_palabra(c[1] & 0xFFFFFF, fmt);
                    }
                }
            }
        } else if (kind == K_ANILLO) {
            int ix0 = p[11], iy0 = p[12], ix1 = p[13], iy1 = p[14];
            bool dentro = false;
            if (x >= x0 && x < x1 && y >= y0 && y < iy0) dentro = true;
            if (x >= x0 && x < x1 && y >= iy1 && y < y1) dentro = true;
            if (x >= x0 && x < ix0 && y >= iy0 && y < iy1) dentro = true;
            if (x >= ix1 && x < x1 && y >= iy0 && y < iy1) dentro = true;
            if (dentro) poner(px, c0, alfa, fmt);
        }
    }
}
