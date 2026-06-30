namespace th06 {
#pragma once
enum { TH_ERR_ECLMANAGER_ENEMY_DATA_CORRUPT=0 };
struct GameErrorContext { template<class...A> void Log(A...){} template<class...A> void Fatal(A...){} };
extern GameErrorContext g_GameErrorContext;
}
