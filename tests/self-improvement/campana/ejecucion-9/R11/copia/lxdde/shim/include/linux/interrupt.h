#ifndef _LX_LINUX_INTERRUPT_H
#define _LX_LINUX_INTERRUPT_H
typedef enum irqreturn { IRQ_NONE = 0, IRQ_HANDLED = 1, IRQ_WAKE_THREAD = 2 } irqreturn_t;
typedef irqreturn_t (*irq_handler_t)(int, void *);
typedef int irq_handler_ret;
#define IRQF_SHARED 0x00000080
#define IRQF_TRIGGER_NONE 0
int lx_request_irq(unsigned irq, irq_handler_t handler, unsigned long flags, const char *name, void *dev);
void lx_free_irq(unsigned irq, void *dev);
#define request_irq(irq, h, f, n, d) lx_request_irq((irq), (h), (f), (n), (d))
#define free_irq(irq, d) lx_free_irq((irq), (d))
#define request_threaded_irq(irq, h, th, f, n, d) lx_request_irq((irq), (h), (f), (n), (d))
#endif
