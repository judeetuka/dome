.section .text
.global dome_cpuid_vendor_stub
.type dome_cpuid_vendor_stub, @function
dome_cpuid_vendor_stub:
    xor %eax, %eax
    ret
