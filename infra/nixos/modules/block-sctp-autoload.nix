{ ... }:
{
  # The November 18 NGINX2Root disclosure includes a separate Linux SCTP flaw.
  # Revisit the block once the fleet is running a kernel with the fix.
  environment.etc."modprobe.d/90-finite-block-sctp.conf".text = ''
    # Block Linux SCTP autoload until the running kernel includes the fix.
    install sctp /run/current-system/sw/bin/false
  '';
}
