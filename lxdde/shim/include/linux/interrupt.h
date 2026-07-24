#ifndef _LX_LINUX_INTERRUPT_H
#define _LX_LINUX_INTERRUPT_H
typedef enum irqreturn { IRQ_NONE = 0, IRQ_HANDLED = 1, IRQ_WAKE_THREAD = 2 } irqreturn_t;
typedef irqreturn_t (*irq_handler_t)(int, void *);
#endif
