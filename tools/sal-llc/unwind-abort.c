/* La std de soso registra frames incluso con panic=abort. No hay unwinding
 * en el compilador Sal; igual que el stub del enlazador nativo wild-soso. */
void __register_frame(const void *frames) { (void)frames; }
