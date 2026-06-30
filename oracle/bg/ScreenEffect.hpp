#pragma once
#include "inttypes.hpp"
namespace th06 {
// Minimal stub: Stage.cpp only calls these from dead-stripped Draw methods.
struct ScreenEffect {
    template <class... A> static void DrawSquare(A...) {}
    template <class... A> static int RegisterChain(A...) { return 0; }
};
}
