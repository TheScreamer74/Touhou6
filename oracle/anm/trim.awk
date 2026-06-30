# Keep only the anm interpreter + loader; delete D3D draw/surface/texture methods
# (stubbed separately). Deletes from "Ret AnmManager::name(" to the next top-level }.
BEGIN { skip = 0 }
{
  if (skip) { if ($0 == "}") skip = 0; next }
  if ($0 ~ /AnmManager::(Draw|Draw2|Draw3|DrawEndingRect|DrawFacingCamera|DrawInner|DrawNoRotation|DrawStringFormat|DrawStringFormat2|DrawTextToSprite|DrawVmTextFmt|CopySurfaceToBackBuffer|SetRenderStateForVm|SetupVertexBuffer|TakeScreenshot|TakeScreenshotIfRequested|TranslateRotation|LoadSurface|ReleaseSurface|ReleaseSurfaces|LoadTexture|CreateEmptyTexture|LoadTextureAlphaChannel|ReleaseTexture)\(/) {
    skip = 1; next
  }
  print
}
