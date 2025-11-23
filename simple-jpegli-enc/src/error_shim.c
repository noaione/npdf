#include "wrapper.h"

#if defined(__clang__) || defined(__GNUC__)
__attribute__((returns_twice))
#endif
int rust_jpegli_setjmp(rust_jpegli_jmp_buf *env) {
    return setjmp(env->inner);
}

void rust_jpegli_longjmp(rust_jpegli_jmp_buf *env, int value) {
    longjmp(env->inner, value);
}
