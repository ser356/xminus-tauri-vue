; Reuse the registered installation, including in silent mode.
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R4 SHCTX "${MANUPRODUCTKEY}" ""
  ${If} $R4 != ""
  ${AndIf} ${FileExists} "$R4\${MAINBINARYNAME}.exe"
    StrCpy $INSTDIR $R4
    SetOutPath "$INSTDIR"
  ${EndIf}
!macroend
