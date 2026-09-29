# glibc llama `write` con la syscall 1 de Linux. En soso el 1 es `read`,
# así que el mensaje acababa escrito encima del propio búfer (a menudo
# `.rodata`). Esta envoltura usa SYS_WRITE (2). Los argumentos ya vienen
# en rdi/rsi/rdx.
.globl __wrap___libc_write
.globl __wrap_write
__wrap___libc_write:
__wrap_write:
	mov $2, %eax
	syscall
	ret
