#pragma once
namespace th06 {
enum ChainCallbackResult { CHAIN_CALLBACK_RESULT_CONTINUE_AND_REMOVE_JOB=0, CHAIN_CALLBACK_RESULT_CONTINUE=1, CHAIN_CALLBACK_RESULT_STOP=2, CHAIN_CALLBACK_RESULT_FAIL=2 };
typedef int (*ChainCallback)(void*);
typedef int (*ChainAddedCallback)(void*);
typedef int (*ChainDeletedCallback)(void*);
struct ChainElem { ChainCallback callback=nullptr; ChainAddedCallback addedCallback=nullptr; ChainDeletedCallback deletedCallback=nullptr; void* arg=nullptr; };
struct Chain { template<class...A> int AddToCalcChain(A...){ return 0; } template<class...A> int AddToDrawChain(A...){ return 0; }
    template<class...A> void Cut(A...){} };
extern Chain g_Chain;
}
