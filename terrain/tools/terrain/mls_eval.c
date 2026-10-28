/* Python版と同じ二次MLSを、近傍リストから倍精度で評価する。 */
#include <math.h>
#include <stdint.h>
#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT
#endif
EXPORT int mls_eval(const double *xy, const double *z, const double *q,
                    const double *radius, const int64_t *offset,
                    const int64_t *ids, int count, double *out) {
    for (int k = 0; k < count; ++k) {
        double a[6][7] = {{0}};
        for (int64_t j = offset[k]; j < offset[k+1]; ++j) {
            int64_t id = ids[j];
            double u = (xy[2*id]-q[2*k])/radius[k];
            double v = (xy[2*id+1]-q[2*k+1])/radius[k];
            double r = sqrt(u*u+v*v), s = fmax(0, 1-r);
            double w = s*s*s*s*(4*r+1), p[6] = {1,u,v,u*u,u*v,v*v};
            for (int row = 0; row < 6; ++row) {
                for (int col = 0; col < 6; ++col) a[row][col] += p[row]*w*p[col];
                a[row][6] += p[row]*w*z[id];
            }
        }
        for (int row = 0; row < 6; ++row) a[row][row] += row < 3 ? 1e-10 : 1e-5;
        for (int col = 0; col < 6; ++col) {
            int pivot = col;
            for (int row = col+1; row < 6; ++row)
                if (fabs(a[row][col]) > fabs(a[pivot][col])) pivot = row;
            if (fabs(a[pivot][col]) < 1e-30) return k+1;
            if (pivot != col) for (int j = col; j < 7; ++j) {
                double t=a[col][j]; a[col][j]=a[pivot][j]; a[pivot][j]=t;
            }
            for (int row = col+1; row < 6; ++row) {
                double f = a[row][col]/a[col][col];
                for (int j = col+1; j < 7; ++j) a[row][j] -= f*a[col][j];
            }
        }
        double answer[6];
        for (int row = 5; row >= 0; --row) {
            double t = a[row][6];
            for (int col = row+1; col < 6; ++col) t -= a[row][col]*answer[col];
            answer[row] = t/a[row][row];
        }
        if (!isfinite(answer[0])) return k+1;
        out[k] = answer[0];
    }
    return 0;
}
