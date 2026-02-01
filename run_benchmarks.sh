#!/bin/bash
# Script to run benchmarks on all VMs and collect results

RESULTS_FILE="/Users/valenta.and.thomas/Developer/firecracker/benchmark_results.txt"

run_benchmark() {
    VM_NAME=$1
    ZONE=$2
    MACHINE_TYPE=$3
    
    echo "Running benchmark on $VM_NAME..."
    
    OUTPUT=$(gcloud compute ssh $VM_NAME --zone=$ZONE --project=e2b-dev --command="
cd ~/firecracker
eval \"\$(mise activate bash)\"
sg docker -c 'tools/devtool -y test -- -m nonci -s integration_tests/performance/test_sysbench_pause_resume.py -v' 2>&1
" 2>&1)
    
    # Extract the three duration values
    NO_UFFD=$(echo "$OUTPUT" | grep -E "^\[normal_no_uffd\] Workload duration:" | awk '{print $4}' | tr -d 's')
    NORMAL_UFFD=$(echo "$OUTPUT" | grep -E "^\[normal_uffd\] Workload duration:" | awk '{print $4}' | tr -d 's')
    HUGEPAGES_UFFD=$(echo "$OUTPUT" | grep -E "^\[hugepages_uffd\] Workload duration:" | awk '{print $4}' | tr -d 's')
    
    if [ -n "$NO_UFFD" ] && [ -n "$NORMAL_UFFD" ] && [ -n "$HUGEPAGES_UFFD" ]; then
        echo "$VM_NAME, $MACHINE_TYPE, $ZONE, $NO_UFFD, $NORMAL_UFFD, $HUGEPAGES_UFFD" >> $RESULTS_FILE
        echo "  Results: no_uffd=$NO_UFFD, normal_uffd=$NORMAL_UFFD, hugepages_uffd=$HUGEPAGES_UFFD"
    else
        echo "  ERROR: Could not extract results for $VM_NAME"
        echo "$VM_NAME, $MACHINE_TYPE, $ZONE, ERROR, ERROR, ERROR" >> $RESULTS_FILE
    fi
}

# Run benchmarks
run_benchmark "c2d" "us-west2-c" "c2d-standard-16"
run_benchmark "c3" "us-west2-b" "c3-standard-22"
run_benchmark "c3d" "us-west4-a" "c3d-standard-16"
run_benchmark "c4" "us-west2-a" "c4-standard-16"
run_benchmark "c4a" "us-west2-b" "c4a-standard-16"
run_benchmark "n2" "us-west2-c" "n2-standard-16"
run_benchmark "n2d" "us-west2-c" "n2d-standard-16"

echo "All benchmarks complete! Results in $RESULTS_FILE"
