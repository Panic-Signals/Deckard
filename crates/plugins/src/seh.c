#include <windows.h>
#include <stdint.h>
#include <malloc.h>

static int deckard_is_hw_fault(DWORD code) {
    switch (code) {
        case EXCEPTION_ACCESS_VIOLATION:
        case EXCEPTION_IN_PAGE_ERROR:
        case EXCEPTION_ARRAY_BOUNDS_EXCEEDED:
        case EXCEPTION_DATATYPE_MISALIGNMENT:
        case EXCEPTION_FLT_DIVIDE_BY_ZERO:
        case EXCEPTION_FLT_INVALID_OPERATION:
        case EXCEPTION_ILLEGAL_INSTRUCTION:
        case EXCEPTION_INT_DIVIDE_BY_ZERO:
        case EXCEPTION_INT_OVERFLOW:
        case EXCEPTION_PRIV_INSTRUCTION:
        case EXCEPTION_STACK_OVERFLOW:
            return EXCEPTION_EXECUTE_HANDLER;
        default:
            return EXCEPTION_CONTINUE_SEARCH;
    }
}

uint32_t deckard_seh_call(void (*fn)(void *), void *ctx, uintptr_t *fault_rip) {
    EXCEPTION_POINTERS *ep = NULL;
    __try {
        fn(ctx);
        return 0;
    } __except (ep = GetExceptionInformation(), deckard_is_hw_fault(GetExceptionCode())) {
        if (fault_rip) {
            *fault_rip = (ep && ep->ContextRecord) ? (uintptr_t)ep->ContextRecord->Rip : 0;
        }
        if (GetExceptionCode() == EXCEPTION_STACK_OVERFLOW) {
            _resetstkoflw();
        }
        return (uint32_t)GetExceptionCode();
    }
}

/* Deliberate C-side null read used by unit tests to verify SEH containment. */
void deckard_seh_test_null_read(void) {
    volatile int *p = (volatile int *)0;
    (void)*p;
}
