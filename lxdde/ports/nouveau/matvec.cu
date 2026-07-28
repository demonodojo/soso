/* G5: matvec f32 (y = W·x) para GB205 (sm_120). Compilar con
 * scripts/l6-g4f-build-sass.sh, igual que saxpy.cu.
 *
 * Un hilo por fila. `rows` es la altura de la TANDA que se lanza, no la de la
 * matriz entera: el staging de G5 no cabe en la sysmem del compute y las filas
 * van por tandas (ver gsp_compute_mv_rows_per_tile). Por eso `w` e `y` apuntan
 * al principio de la tanda y el kernel no sabe nada del troceado.
 *
 * Sin memoria compartida ni reducciones entre hilos a propósito: cada hilo lee
 * su fila entera y escribe un solo float. Es la versión lenta, y es la que se
 * puede depurar cuando lo único que se ve del otro lado es un semáforo. */
extern "C" __global__ void matvec_f32(const float *w, const float *x, float *y,
                                      int rows, int cols)
{
    int r = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    if (r >= rows) {
        return;
    }

    const float *row = w + (long)r * (long)cols;
    float sum = 0.0f;

    for (int c = 0; c < cols; c++) {
        sum += row[c] * x[c];
    }
    y[r] = sum;
}
