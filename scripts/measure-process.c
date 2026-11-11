#include <libproc.h>
#include <sys/resource.h>
#include <stdio.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    struct rusage_info_v4 info = {0};
    int pid = atoi(argv[1]);
    if (proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)&info)) {
        perror("proc_pid_rusage"); return 1;
    }
    printf("pid=%d rss_mib=%.3f footprint_mib=%.3f lifetime_peak_mib=%.3f\n", pid,
        info.ri_resident_size / 1048576., info.ri_phys_footprint / 1048576.,
        info.ri_lifetime_max_phys_footprint / 1048576.);
}
