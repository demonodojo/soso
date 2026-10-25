/* LLVM IR -> ELF x86-64 for Sal. No C frontend, driver subprocess or shell. */
#include <llvm-c/Core.h>
#include <llvm-c/IRReader.h>
#include <llvm-c/Analysis.h>
#include <llvm-c/Target.h>
#include <llvm-c/TargetMachine.h>
#include <stddef.h>
extern void sal_llc_message(const char *, int);
#include <string.h>
#ifndef SAL_LLC_ID
#define SAL_LLC_ID "development"
#endif

int herramienta_main(int argc, const char **argv) {
    const char *input = NULL, *output = NULL;
    LLVMCodeGenOptLevel opt = LLVMCodeGenLevelNone;
    for (int i = 1; i < argc; i++) {
        const char *a = argv[i];
        if (!strcmp(a, "--version")) { sal_llc_message("sal-llc 1 (LLVM 19, x86_64 soso) " SAL_LLC_ID, 0); return 0; }
        if (!strcmp(a, "-o") && i + 1 < argc) { output = argv[++i]; continue; }
        if (!strcmp(a, "-x") && i + 1 < argc && !strcmp(argv[i+1], "ir")) { i++; continue; }
        if (!strcmp(a, "-O3")) { opt = LLVMCodeGenLevelAggressive; continue; }
        if (!strcmp(a, "-c") || !strcmp(a, "-O0") || !strcmp(a, "-g") ||
            !strcmp(a, "--target=x86_64-unknown-none-elf") || !strcmp(a, "-ffreestanding") ||
            !strcmp(a, "-fno-stack-protector") || !strcmp(a, "-mno-red-zone") || !strcmp(a, "-fno-pic")) continue;
        if (a[0] == '-' || input) { sal_llc_message(a, 1); return 1; }
        input = a;
    }
    if (!input || !output) { sal_llc_message("uso: sal-llc -x ir fuente.ll -c -o destino.o", 1); return 1; }
    LLVMInitializeX86TargetInfo(); LLVMInitializeX86Target();
    LLVMInitializeX86TargetMC(); LLVMInitializeX86AsmPrinter();
    LLVMContextRef ctx = LLVMContextCreate();
    LLVMMemoryBufferRef buf;
    LLVMModuleRef mod;
    char *error = NULL;
    if (LLVMCreateMemoryBufferWithContentsOfFile(input, &buf, &error) ||
        LLVMParseIRInContext(ctx, buf, &mod, &error)) goto fail;
    if (LLVMVerifyModule(mod, LLVMReturnStatusAction, &error)) goto fail;
    const char *triple = "x86_64-unknown-none-elf";
    LLVMTargetRef target;
    if (LLVMGetTargetFromTriple(triple, &target, &error)) goto fail;
    LLVMTargetMachineRef machine = LLVMCreateTargetMachine(target, triple, "x86-64", "",
        opt, LLVMRelocStatic, LLVMCodeModelSmall);
    if (!machine) { error = LLVMCreateMessage("no se puede crear el destino"); goto fail; }
    LLVMSetTarget(mod, triple);
    LLVMTargetDataRef data = LLVMCreateTargetDataLayout(machine);
    LLVMSetModuleDataLayout(mod, data);
    for (LLVMValueRef f = LLVMGetFirstFunction(mod); f; f = LLVMGetNextFunction(f)) {
        LLVMAddAttributeAtIndex(f, LLVMAttributeFunctionIndex,
            LLVMCreateEnumAttribute(ctx, LLVMGetEnumAttributeKindForName("noredzone", 9), 0));
    }
    if (LLVMTargetMachineEmitToFile(machine, mod, (char *)output, LLVMObjectFile, &error)) goto fail;
    LLVMDisposeTargetData(data); LLVMDisposeTargetMachine(machine);
    LLVMDisposeModule(mod); LLVMContextDispose(ctx);
    return 0;
fail:
    sal_llc_message(error ? error : "error LLVM", 1);
    if (error) LLVMDisposeMessage(error);
    return 1;
}
