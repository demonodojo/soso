/* G4f: kernel SAXPY mínimo para GB205 (sm_120). Compilar con scripts/l6-g4f-build-sass.sh */
extern "C" __global__ void saxpy(float a, const float *x, float *y, int n)
{
    int i = (int)(blockIdx.x * blockDim.x + threadIdx.x);
    if (i >= n) {
        return;
    }
    y[i] = a * x[i] + y[i];
}
