// G1: backend GPU de la escena de primitivas del escritorio (GPRIM / GSCEN v2).
// Un hilo por píxel recorre la lista en orden, así que el resultado es idéntico al backend
// CPU de user/sal/escritorio/escena.sal (esc_cpu_ejecutar). Formato de primitiva: 16 palabras
// de 32 bits; ver crates/soso-wm-proto/src/gprim.rs. Sin cabeceras de CUDA (NVRTC).

#define K_SOLIDO 1
#define K_DEGRADADO 2
#define K_RRECT 3
#define K_ANILLO 4
#define K_REJILLA 5

__device__ __forceinline__ int rr_inset(int radio, int d) {
    int k = 0;
    if (d < radio) {
        int dy = radio - d;
        while (k < radio) {
            int dx = radio - k;
            if (dx * dx + dy * dy > radio * radio) k++; else break;
        }
    }
    return k;
}

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

// ¿(x,y) cae dentro del rectángulo x0..x1 × y0..y1 con esas esquinas redondeadas?
__device__ __forceinline__ bool en_rrect(int x, int y, int x0, int y0, int x1, int y1,
                                         int rtl, int rtr, int rbr, int rbl) {
    int w = x1 - x0, h = y1 - y0;
    if (w <= 0 || h <= 0) return false;
    if (x < x0 || x >= x1 || y < y0 || y >= y1) return false;
    int m = imin(w / 2, h / 2);
    int a = imin(rtl, m), b = imin(rtr, m), c = imin(rbr, m), d = imin(rbl, m);
    int fila = y - y0;
    int il = 0, ir = 0;
    if (fila < a) il = rr_inset(a, fila);
    if (h - 1 - fila < d) il = imax(il, rr_inset(d, h - 1 - fila));
    if (fila < b) ir = rr_inset(b, fila);
    if (h - 1 - fila < c) ir = imax(ir, rr_inset(c, h - 1 - fila));
    return x >= x0 + il && x < x1 - ir;
}

extern "C" __global__ void gprim(unsigned* rt, int pitch_px, int ancho, int alto, int fmt,
                                 const int* prims, int n,
                                 const unsigned* celdas, const unsigned char* psf) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;
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
            unsigned rd = (unsigned)p[8];
            int rtl = rd & 255, rtr = (rd >> 8) & 255, rbr = (rd >> 16) & 255, rbl = (rd >> 24) & 255;
            int bw = p[10];
            if (x < 0 || x >= ancho || y < 0 || y >= alto) continue;
            if (bw > 0) {
                if (en_rrect(x, y, x0, y0, x1, y1, rtl, rtr, rbr, rbl))
                    *px = a_palabra((unsigned)p[9] & 0xFFFFFF, fmt);
                if (en_rrect(x, y, x0 + bw, y0 + bw, x1 - bw, y1 - bw,
                             imax(0, rtl - bw), imax(0, rtr - bw), imax(0, rbr - bw), imax(0, rbl - bw)))
                    poner(px, c0, alfa, fmt);
            } else if (en_rrect(x, y, x0, y0, x1, y1, rtl, rtr, rbr, rbl)) {
                poner(px, c0, alfa, fmt);
            }
        } else if (kind == K_REJILLA) {
            // Rejilla de celdas de texto: fondo opaco por celda y tinta donde el glifo tiene bit.
            int cols = p[11], filas = p[12], gw = p[13], gh = p[14], bpg = p[8], hdr = p[9], c0i = p[15];
            int rx = x - x0, ry = y - y0;
            if (rx >= 0 && ry >= 0 && gw > 0 && gh > 0 && rx < cols * gw && ry < filas * gh) {
                int cx = rx / gw, col = rx % gw, cy = ry / gh, row = ry % gh;
                const unsigned* c = celdas + (size_t)(c0i + cy * cols + cx) * 3;
                *px = a_palabra(c[2] & 0xFFFFFF, fmt);
                if (c[0] != 0xFFFFFFFFu) {
                    int bpr = (gw + 7) / 8;
                    unsigned v = psf[hdr + c[0] * bpg + row * bpr + col / 8];
                    if ((v >> (7 - (col & 7))) & 1) *px = a_palabra(c[1] & 0xFFFFFF, fmt);
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
