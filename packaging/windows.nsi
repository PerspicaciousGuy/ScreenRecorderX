Unicode true
!include "MUI2.nsh"
!ifndef STAGE
  !error "Set /DSTAGE to the audited application staging directory"
!endif
Name "Clearcast"
OutFile "..\dist\Clearcast-0.1.0-windows-x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Clearcast"
RequestExecutionLevel user
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "Clearcast"
  SetOutPath "$INSTDIR"
  File /r /x "gstreamer" "${STAGE}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Clearcast"
  CreateShortcut "$SMPROGRAMS\Clearcast\Clearcast.lnk" "$INSTDIR\clearcast.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Clearcast" "DisplayName" "Clearcast"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Clearcast" "UninstallString" '$"$INSTDIR\Uninstall.exe$"'
SectionEnd
Section "Uninstall"
  RMDir /r "$INSTDIR"
  RMDir /r "$SMPROGRAMS\Clearcast"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Clearcast"
  ; Per-user settings and recovery journals are intentionally preserved.
SectionEnd
