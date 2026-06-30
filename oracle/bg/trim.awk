# Delete Stage:: methods not needed by the bg-state oracle (keep OnUpdate +
# UpdateObjects). Deletes from a matching "Ret Stage::name(" line to the next
# top-level "}" line inclusive. Also drops the preceding #pragma var_order line.
BEGIN { skip = 0 }
{
    if (skip) { if ($0 == "}") skip = 0; next }
    if ($0 ~ /Stage::(OnDrawHighPrio|OnDrawLowPrio|AddedCallback|RegisterChain|DeletedCallback|LoadStageData|RenderObjects|CutChain)\(/) {
        # drop a pragma line we may have just printed
        skip = 1; next
    }
    print
}
