#include <stddef.h>
#include <stdio.h>
#include <setjmp.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct rust_jpegli_jmp_buf {
	jmp_buf inner;
} rust_jpegli_jmp_buf;

#if defined(__clang__) || defined(__GNUC__)
__attribute__((returns_twice))
#endif
int rust_jpegli_setjmp(rust_jpegli_jmp_buf *env);
void rust_jpegli_longjmp(rust_jpegli_jmp_buf *env, int value);

#ifdef __cplusplus
}
#endif
