# Start the installer on the first console, once per boot. Shared by the ISO and
# the interactive VM so the guard cannot drift between them.
{ installer }:
{
  environment.systemPackages = [ installer ];
  programs.bash.loginShellInit = ''
    if [ "$(tty)" = /dev/tty1 ] && [ -z "$NUN_STARTED" ]; then
      export NUN_STARTED=1
      nun-installer
    fi
  '';
}
