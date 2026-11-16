/* La std de soso registra frames incluso con panic=abort. No hay unwinding
 * en el compilador Sal; igual que el stub del enlazador nativo wild-soso. */
void __register_frame(const void *frames) { (void)frames; }

/* glibc estática referencia esto desde qsort aunque el proceso no habilite
 * cancelación ni excepciones. Un unwind sería un error fatal en panic=abort. */
int __gcc_personality_v0(int version, int actions, unsigned long exception_class,
                         void *exception, void *context) {
    (void)version;
    (void)actions;
    (void)exception_class;
    (void)exception;
    (void)context;
    __builtin_trap();
}

void _Unwind_Resume(void *exception) { (void)exception; __builtin_trap(); }
