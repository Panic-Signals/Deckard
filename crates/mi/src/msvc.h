// Force-included under MSVC: stmlib marks its inline helpers with GCC's
// `__attribute__((always_inline))`, which MSVC doesn't have. They're hints; drop them.

#pragma once

#define __attribute__(x)
