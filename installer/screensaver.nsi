; Photo Stamp screensaver setup: installs photo-stamp-screensaver.scr into System32 (the only place
; Screen Saver Settings lists screensavers from, hence admin rights), makes it this user's
; screensaver and opens Screen Saver Settings, where "Settings..." sets the photo folder and the
; place server.
;
; Built on Linux by scripts/build-windows.sh:
;   makensis -WX -DVERSION=0.1.0 -DVIVERSION=0.1.0.0 -DSCR=../dist/photo-stamp-screensaver.scr \
;            -DTOWNS=../dist/towns.sqlite -DOUTFILE=../dist/photo-stamp-screensaver-setup.exe \
;            installer/screensaver.nsi
;
; Silent install (tests): photo-stamp-screensaver-setup.exe /S (doesn't open Screen Saver Settings).
;
; Setup also has the .scr schedule its automatic updates (`/install`: a SYSTEM task, and the
; installing user's place server copied machine-wide); uninstalling undoes that (`/uninstall`). The
; users' own settings (HKCU\Software\Photo Stamp Screensaver) and the place caches in the photo
; folders are kept on uninstall, as are the photos.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!ifndef VERSION
  !define VERSION "0.0.0-dev"
!endif
!ifndef VIVERSION
  !define VIVERSION "0.0.0.0"
!endif
!ifndef SCR
  !define SCR "../dist/photo-stamp-screensaver.scr"
!endif
!ifndef TOWNS
  !define TOWNS "../dist/towns.sqlite"
!endif
!ifndef OUTFILE
  !define OUTFILE "../dist/photo-stamp-screensaver-setup.exe"
!endif

!define APP "Photo Stamp screensaver"
!define SCRNAME "photo-stamp-screensaver.scr"
; Setup is 32-bit: $SYSDIR would be SysWOW64. This is the real System32, with redirection off.
!define SCRPATH "$WINDIR\System32\${SCRNAME}"
!define UKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\PhotoStampScreensaver"
!define DESKTOP "Control Panel\Desktop"
!define SHORTCUT "$SMPROGRAMS\Photo Stamp screensaver settings.lnk"

Name "${APP}"
OutFile "${OUTFILE}"
RequestExecutionLevel admin
; The uninstaller and the towns database (places while the place server can't be reached) live
; here; the screensaver itself is in System32.
InstallDir "$PROGRAMFILES64\Photo Stamp Screensaver"
SetCompressor /SOLID lzma
BrandingText "${APP} ${VERSION}"
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${VIVERSION}"
VIFileVersion "${VIVERSION}"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "${APP} setup"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT License"

!define MUI_ICON "../assets/photo-stamp.ico"
!define MUI_UNICON "../assets/photo-stamp.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "This installs a screensaver that shows your photos, with where and when each one was taken.$\r$\n$\r$\nYour photos are never changed.$\r$\n$\r$\nWindows will ask for permission, because screensavers are installed for everyone on this PC."
!define MUI_FINISHPAGE_TEXT "The Photo Stamp screensaver is installed and turned on.$\r$\n$\r$\nIn Screen Saver Settings, click Settings... to choose your photo folder."
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Open Screen Saver Settings"
!define MUI_FINISHPAGE_RUN_FUNCTION OpenScreenSaverSettings
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "The Photo Stamp screensaver needs 64-bit Windows." /SD IDOK
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Function un.onInit
  SetRegView 64
FunctionEnd

Section "Install"
  ; A running copy (the screensaver, or its settings) holds the file: NSIS asks to retry.
  ${DisableX64FSRedirection}
  SetOutPath "$WINDIR\System32"
  File "/oname=${SCRNAME}" "${SCR}"
  ${EnableX64FSRedirection}

  SetOutPath "$INSTDIR"
  File "/oname=towns.sqlite" "${TOWNS}"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "${UKEY}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${UKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UKEY}" "Publisher" "Photo Stamp"
  WriteRegStr HKLM "${UKEY}" "DisplayIcon" "${SCRPATH},0"
  WriteRegStr HKLM "${UKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UKEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UKEY}" "NoRepair" 1
  WriteRegDWORD HKLM "${UKEY}" "EstimatedSize" 20000

  SetShellVarContext all
  CreateShortcut "${SHORTCUT}" "${SCRPATH}" "/c" "${SCRPATH}" 0

  ; Automatic updates: a scheduled task (as SYSTEM), using this user's place server if they have one
  ; (or photo-stamp's). The .scr does it, so it's the same code the settings dialog uses.
  ${DisableX64FSRedirection}
  ExecWait '"${SCRPATH}" /install' $0
  ${EnableX64FSRedirection}
  ${If} $0 != 0
    DetailPrint "Automatic updates couldn't be scheduled ($0)."
  ${EndIf}

  ; This user's screensaver from now on. (Setup runs elevated: on a standard account that
  ; borrowed an administrator's password, this is the administrator's; Screen Saver Settings,
  ; opened at the end, sets it for the right person either way.)
  WriteRegStr HKCU "${DESKTOP}" "SCRNSAVE.EXE" "${SCRPATH}"
  WriteRegStr HKCU "${DESKTOP}" "ScreenSaveActive" "1"
SectionEnd

; What right-click > Install does for a .scr: makes it the screensaver and opens Screen Saver
; Settings on it. The 64-bit rundll32, so the dialog lists System32's screensavers.
Function OpenScreenSaverSettings
  ${DisableX64FSRedirection}
  Exec '"$WINDIR\System32\rundll32.exe" desk.cpl,InstallScreenSaver ${SCRPATH}'
  ${EnableX64FSRedirection}
FunctionEnd

Section "Uninstall"
  ; No more update task or machine-wide update settings (users' own settings stay).
  ${DisableX64FSRedirection}
  ExecWait '"${SCRPATH}" /uninstall'
  ; Windows may have recorded the screensaver by its short (8.3) name; compare long names.
  ReadRegStr $0 HKCU "${DESKTOP}" "SCRNSAVE.EXE"
  ${If} $0 != ""
    System::Call 'kernel32::GetLongPathNameW(w r0, w .r1, i ${NSIS_MAX_STRLEN}) i .r2'
    ${If} $2 > 0
      StrCpy $0 $1
    ${EndIf}
  ${EndIf}
  ; StrCmp is case-insensitive.
  ${If} $0 == "${SCRPATH}"
    DeleteRegValue HKCU "${DESKTOP}" "SCRNSAVE.EXE"
  ${EndIf}
  Delete /REBOOTOK "${SCRPATH}"
  ; What updates leave behind.
  Delete /REBOOTOK "${SCRPATH}.old"
  Delete "${SCRPATH}.new"
  ${EnableX64FSRedirection}

  SetShellVarContext all
  Delete "${SHORTCUT}"
  Delete "$INSTDIR\towns.sqlite"
  Delete "$INSTDIR\towns.sqlite.old"
  Delete "$INSTDIR\towns.sqlite.new"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "${UKEY}"
SectionEnd
