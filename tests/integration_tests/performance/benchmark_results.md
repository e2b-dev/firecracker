# Firecracker Snapshot/Restore Benchmark

dd 1 GiB to `/dev/shm` after full snapshot/restore. 2 vCPUs, 2048 MiB RAM. 10 runs per machine.

## Overview (seconds, avg of 10 runs)

| Machine | Virt | CPU | 4K | 4K+dirty | 4K+uffd | 2M+uffd |
|---------|------|-----|----|----------|---------|---------|
| c6i.metal (AWS) | metal | Xeon 8375C Ice Lake | **1.16** | **1.19** | **4.83** | **0.48** |
| c3-192-metal (GCP) | metal | Xeon 8481C Sapphire Rapids | 1.41 | 1.43 | 7.26 | 0.56 |
| c4-288-metal (GCP) | metal | Xeon 6985P-C Emerald Rapids | 1.41 | 1.39 | 10.07 | 0.54 |
| m5.metal (AWS) | metal | Xeon 8259CL Cascade Lake | 1.47 | 1.48 | 6.78 | 0.63 |
| c4-standard-96 (GCP) | nested | Xeon 8581C Emerald Rapids | 3.06 | 3.05 | 8.38 | 0.48 |
| c4-standard-288 (GCP) | nested | Xeon 6985P-C Emerald Rapids | 3.44 | 3.41 | 13.18 | 0.50 |
| c3-standard-88 (GCP) | nested | Xeon 8481C Sapphire Rapids | 3.57 | 3.57 | 9.20 | 0.48 |
| c3-standard-176 (GCP) | nested | Xeon 8481C Sapphire Rapids | 3.64 | 3.62 | 9.28 | 0.48 |
| n2-standard-96 (GCP) | nested | Xeon @ 2.60GHz Cascade Lake | 4.85 | 4.82 | 11.13 | 0.49 |

All machines: Ubuntu 22.04.5 LTS. GCP kernel 6.8.0-1047-gcp, AWS kernel 6.8.0-1044-aws.

## n2-standard-96 — GCP nested, Xeon @ 2.60GHz (Cascade Lake)

### normal_no_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 4.848 | 0.042 |
| Throughput (MiB/s) | 211.3 | 1.8 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 4.875 | 210.1 |
| 2 | 4.846 | 211.3 |
| 3 | 4.919 | 208.2 |
| 4 | 4.877 | 210.0 |
| 5 | 4.790 | 213.8 |
| 6 | 4.796 | 213.5 |
| 7 | 4.816 | 212.6 |
| 8 | 4.801 | 213.3 |
| 9 | 4.873 | 210.1 |
| 10 | 4.884 | 209.7 |

</details>

### normal_no_uffd_dirty_tracking
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 4.819 | 0.043 |
| Throughput (MiB/s) | 212.5 | 1.9 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 4.776 | 214.4 |
| 2 | 4.793 | 213.6 |
| 3 | 4.857 | 210.8 |
| 4 | 4.859 | 210.7 |
| 5 | 4.882 | 209.7 |
| 6 | 4.783 | 214.1 |
| 7 | 4.885 | 209.6 |
| 8 | 4.785 | 214.0 |
| 9 | 4.787 | 213.9 |
| 10 | 4.782 | 214.1 |

</details>

### normal_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 11.127 | 0.736 |
| Throughput (MiB/s) | 92.4 | 5.9 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 12.296 | 83.3 |
| 2 | 10.698 | 95.7 |
| 3 | 10.897 | 94.0 |
| 4 | 10.559 | 97.0 |
| 5 | 11.789 | 86.9 |
| 6 | 10.360 | 98.8 |
| 7 | 11.438 | 89.5 |
| 8 | 12.331 | 83.0 |
| 9 | 10.473 | 97.8 |
| 10 | 10.428 | 98.2 |

</details>

### hugepages_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 0.487 | 0.015 |
| Throughput (MiB/s) | 2106.4 | 61.8 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 0.474 | 2158.1 |
| 2 | 0.473 | 2165.0 |
| 3 | 0.479 | 2137.5 |
| 4 | 0.504 | 2033.7 |
| 5 | 0.482 | 2125.4 |
| 6 | 0.479 | 2136.5 |
| 7 | 0.475 | 2157.5 |
| 8 | 0.514 | 1993.5 |
| 9 | 0.508 | 2016.9 |
| 10 | 0.478 | 2140.3 |

</details>

## c3-standard-88 — GCP nested, Xeon 8481C (Sapphire Rapids)

### normal_no_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 3.572 | 0.020 |
| Throughput (MiB/s) | 286.7 | 1.6 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 3.555 | 288.0 |
| 2 | 3.563 | 287.4 |
| 3 | 3.554 | 288.2 |
| 4 | 3.568 | 287.0 |
| 5 | 3.599 | 284.5 |
| 6 | 3.570 | 286.8 |
| 7 | 3.601 | 284.4 |
| 8 | 3.600 | 284.4 |
| 9 | 3.570 | 286.8 |
| 10 | 3.542 | 289.1 |

</details>

### normal_no_uffd_dirty_tracking
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 3.568 | 0.031 |
| Throughput (MiB/s) | 287.0 | 2.5 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 3.619 | 282.9 |
| 2 | 3.532 | 289.9 |
| 3 | 3.607 | 283.9 |
| 4 | 3.597 | 284.7 |
| 5 | 3.539 | 289.4 |
| 6 | 3.553 | 288.2 |
| 7 | 3.552 | 288.3 |
| 8 | 3.550 | 288.5 |
| 9 | 3.591 | 285.1 |
| 10 | 3.538 | 289.4 |

</details>

### normal_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 9.203 | 0.103 |
| Throughput (MiB/s) | 111.3 | 1.2 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 9.231 | 110.9 |
| 2 | 9.188 | 111.5 |
| 3 | 9.030 | 113.4 |
| 4 | 9.276 | 110.4 |
| 5 | 9.363 | 109.4 |
| 6 | 9.172 | 111.6 |
| 7 | 9.333 | 109.7 |
| 8 | 9.241 | 110.8 |
| 9 | 9.069 | 112.9 |
| 10 | 9.125 | 112.2 |

</details>

### hugepages_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 0.476 | 0.007 |
| Throughput (MiB/s) | 2152.0 | 28.8 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 0.469 | 2181.3 |
| 2 | 0.473 | 2164.5 |
| 3 | 0.482 | 2126.5 |
| 4 | 0.489 | 2093.6 |
| 5 | 0.469 | 2184.2 |
| 6 | 0.474 | 2158.8 |
| 7 | 0.479 | 2137.6 |
| 8 | 0.474 | 2160.8 |
| 9 | 0.468 | 2186.5 |
| 10 | 0.482 | 2126.5 |

</details>

## c3-standard-176 — GCP nested, Xeon 8481C (Sapphire Rapids)

### normal_no_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 3.637 | 0.038 |
| Throughput (MiB/s) | 281.6 | 2.9 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 3.623 | 282.6 |
| 2 | 3.693 | 277.3 |
| 3 | 3.609 | 283.7 |
| 4 | 3.676 | 278.6 |
| 5 | 3.679 | 278.4 |
| 6 | 3.617 | 283.1 |
| 7 | 3.591 | 285.1 |
| 8 | 3.592 | 285.1 |
| 9 | 3.682 | 278.1 |
| 10 | 3.611 | 283.6 |

</details>

### normal_no_uffd_dirty_tracking
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 3.617 | 0.045 |
| Throughput (MiB/s) | 283.1 | 3.5 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 3.607 | 283.9 |
| 2 | 3.587 | 285.4 |
| 3 | 3.700 | 276.7 |
| 4 | 3.590 | 285.3 |
| 5 | 3.593 | 285.0 |
| 6 | 3.574 | 286.5 |
| 7 | 3.585 | 285.7 |
| 8 | 3.588 | 285.4 |
| 9 | 3.655 | 280.2 |
| 10 | 3.691 | 277.4 |

</details>

### normal_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 9.282 | 0.315 |
| Throughput (MiB/s) | 110.5 | 3.5 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 9.251 | 110.7 |
| 2 | 10.201 | 100.4 |
| 3 | 9.188 | 111.5 |
| 4 | 9.202 | 111.3 |
| 5 | 9.032 | 113.4 |
| 6 | 9.211 | 111.2 |
| 7 | 9.252 | 110.7 |
| 8 | 9.166 | 111.7 |
| 9 | 9.265 | 110.5 |
| 10 | 9.056 | 113.1 |

</details>

### hugepages_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 0.484 | 0.024 |
| Throughput (MiB/s) | 2119.5 | 99.3 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 0.473 | 2164.0 |
| 2 | 0.469 | 2182.3 |
| 3 | 0.535 | 1912.8 |
| 4 | 0.476 | 2150.0 |
| 5 | 0.464 | 2205.5 |
| 6 | 0.478 | 2141.5 |
| 7 | 0.471 | 2172.6 |
| 8 | 0.529 | 1935.4 |
| 9 | 0.471 | 2175.4 |
| 10 | 0.475 | 2155.9 |

</details>

## c3-standard-192-metal — GCP metal, Xeon 8481C (Sapphire Rapids)

### normal_no_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 1.409 | 0.041 |
| Throughput (MiB/s) | 727.4 | 20.8 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 1.440 | 711.2 |
| 2 | 1.362 | 751.8 |
| 3 | 1.422 | 720.2 |
| 4 | 1.378 | 743.3 |
| 5 | 1.368 | 748.7 |
| 6 | 1.364 | 750.6 |
| 7 | 1.474 | 694.5 |
| 8 | 1.436 | 712.9 |
| 9 | 1.374 | 745.4 |
| 10 | 1.472 | 695.5 |

</details>

### normal_no_uffd_dirty_tracking
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 1.430 | 0.031 |
| Throughput (MiB/s) | 716.5 | 15.8 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 1.381 | 741.3 |
| 2 | 1.450 | 706.1 |
| 3 | 1.446 | 708.1 |
| 4 | 1.456 | 703.1 |
| 5 | 1.443 | 709.4 |
| 6 | 1.448 | 707.2 |
| 7 | 1.387 | 738.4 |
| 8 | 1.380 | 741.8 |
| 9 | 1.437 | 712.6 |
| 10 | 1.469 | 697.2 |

</details>

### normal_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 7.263 | 0.597 |
| Throughput (MiB/s) | 142.0 | 11.4 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 6.487 | 157.9 |
| 2 | 6.485 | 157.9 |
| 3 | 7.719 | 132.7 |
| 4 | 8.537 | 119.9 |
| 5 | 7.426 | 137.9 |
| 6 | 6.641 | 154.2 |
| 7 | 7.170 | 142.8 |
| 8 | 7.725 | 132.6 |
| 9 | 7.356 | 139.2 |
| 10 | 7.081 | 144.6 |

</details>

### hugepages_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 0.560 | 0.028 |
| Throughput (MiB/s) | 1834.7 | 87.8 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 0.534 | 1917.3 |
| 2 | 0.535 | 1914.0 |
| 3 | 0.595 | 1722.4 |
| 4 | 0.595 | 1719.7 |
| 5 | 0.541 | 1893.5 |
| 6 | 0.534 | 1918.1 |
| 7 | 0.594 | 1722.7 |
| 8 | 0.538 | 1902.5 |
| 9 | 0.599 | 1709.4 |
| 10 | 0.531 | 1926.9 |

</details>

## c4-standard-96 — GCP nested, Xeon 8581C (Emerald Rapids)

### normal_no_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 3.058 | 0.017 |
| Throughput (MiB/s) | 334.9 | 1.8 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 3.043 | 336.5 |
| 2 | 3.085 | 332.0 |
| 3 | 3.068 | 333.8 |
| 4 | 3.079 | 332.5 |
| 5 | 3.043 | 336.5 |
| 6 | 3.049 | 335.8 |
| 7 | 3.058 | 334.8 |
| 8 | 3.074 | 333.2 |
| 9 | 3.039 | 336.9 |
| 10 | 3.040 | 336.8 |

</details>

### normal_no_uffd_dirty_tracking
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 3.047 | 0.014 |
| Throughput (MiB/s) | 336.1 | 1.5 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 3.053 | 335.4 |
| 2 | 3.031 | 337.8 |
| 3 | 3.068 | 333.8 |
| 4 | 3.067 | 333.9 |
| 5 | 3.040 | 336.9 |
| 6 | 3.065 | 334.1 |
| 7 | 3.042 | 336.7 |
| 8 | 3.041 | 336.8 |
| 9 | 3.032 | 337.7 |
| 10 | 3.035 | 337.4 |

</details>

### normal_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 8.378 | 0.083 |
| Throughput (MiB/s) | 122.2 | 1.2 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 8.317 | 123.1 |
| 2 | 8.331 | 122.9 |
| 3 | 8.459 | 121.0 |
| 4 | 8.409 | 121.8 |
| 5 | 8.365 | 122.4 |
| 6 | 8.321 | 123.1 |
| 7 | 8.560 | 119.6 |
| 8 | 8.289 | 123.5 |
| 9 | 8.439 | 121.3 |
| 10 | 8.291 | 123.5 |

</details>

### hugepages_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 0.478 | 0.016 |
| Throughput (MiB/s) | 2143.6 | 70.7 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 0.496 | 2065.8 |
| 2 | 0.484 | 2115.6 |
| 3 | 0.500 | 2047.8 |
| 4 | 0.460 | 2227.4 |
| 5 | 0.500 | 2046.7 |
| 6 | 0.478 | 2142.7 |
| 7 | 0.474 | 2159.9 |
| 8 | 0.471 | 2173.0 |
| 9 | 0.453 | 2260.5 |
| 10 | 0.466 | 2196.4 |

</details>

## c4-standard-288 — GCP nested, Xeon 6985P-C (Emerald Rapids)

### normal_no_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 3.439 | 0.042 |
| Throughput (MiB/s) | 297.8 | 3.7 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 3.421 | 299.3 |
| 2 | 3.370 | 303.8 |
| 3 | 3.445 | 297.3 |
| 4 | 3.477 | 294.5 |
| 5 | 3.412 | 300.1 |
| 6 | 3.456 | 296.3 |
| 7 | 3.450 | 296.8 |
| 8 | 3.485 | 293.9 |
| 9 | 3.502 | 292.4 |
| 10 | 3.375 | 303.4 |

</details>

### normal_no_uffd_dirty_tracking
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 3.407 | 0.055 |
| Throughput (MiB/s) | 300.6 | 5.0 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 3.424 | 299.1 |
| 2 | 3.255 | 314.6 |
| 3 | 3.416 | 299.8 |
| 4 | 3.414 | 299.9 |
| 5 | 3.396 | 301.6 |
| 6 | 3.434 | 298.2 |
| 7 | 3.400 | 301.2 |
| 8 | 3.421 | 299.3 |
| 9 | 3.439 | 297.8 |
| 10 | 3.474 | 294.8 |

</details>

### normal_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 13.182 | 0.235 |
| Throughput (MiB/s) | 77.7 | 1.4 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 13.223 | 77.4 |
| 2 | 13.752 | 74.5 |
| 3 | 12.990 | 78.8 |
| 4 | 13.162 | 77.8 |
| 5 | 13.189 | 77.6 |
| 6 | 13.200 | 77.6 |
| 7 | 13.013 | 78.7 |
| 8 | 12.817 | 79.9 |
| 9 | 13.337 | 76.8 |
| 10 | 13.136 | 78.0 |

</details>

### hugepages_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 0.496 | 0.009 |
| Throughput (MiB/s) | 2064.8 | 36.9 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 0.490 | 2090.1 |
| 2 | 0.489 | 2093.7 |
| 3 | 0.489 | 2092.8 |
| 4 | 0.494 | 2073.0 |
| 5 | 0.509 | 2011.4 |
| 6 | 0.492 | 2081.5 |
| 7 | 0.489 | 2093.7 |
| 8 | 0.489 | 2094.5 |
| 9 | 0.512 | 2000.9 |
| 10 | 0.508 | 2016.0 |

</details>

## c4-standard-288-metal — GCP metal, Xeon 6985P-C (Emerald Rapids)

### normal_no_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 1.407 | 0.044 |
| Throughput (MiB/s) | 728.4 | 21.3 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 1.375 | 744.7 |
| 2 | 1.494 | 685.5 |
| 3 | 1.381 | 741.4 |
| 4 | 1.470 | 696.7 |
| 5 | 1.469 | 697.0 |
| 6 | 1.385 | 739.1 |
| 7 | 1.380 | 742.2 |
| 8 | 1.364 | 750.8 |
| 9 | 1.382 | 740.7 |
| 10 | 1.373 | 745.7 |

</details>

### normal_no_uffd_dirty_tracking
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 1.390 | 0.025 |
| Throughput (MiB/s) | 737.2 | 13.5 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 1.385 | 739.3 |
| 2 | 1.382 | 741.1 |
| 3 | 1.374 | 745.3 |
| 4 | 1.417 | 722.8 |
| 5 | 1.373 | 746.0 |
| 6 | 1.385 | 739.2 |
| 7 | 1.379 | 742.6 |
| 8 | 1.453 | 704.8 |
| 9 | 1.369 | 748.1 |
| 10 | 1.379 | 742.5 |

</details>

### normal_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 10.069 | 0.286 |
| Throughput (MiB/s) | 101.8 | 2.8 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 10.361 | 98.8 |
| 2 | 10.055 | 101.8 |
| 3 | 9.839 | 104.1 |
| 4 | 10.315 | 99.3 |
| 5 | 9.690 | 105.7 |
| 6 | 10.426 | 98.2 |
| 7 | 10.348 | 99.0 |
| 8 | 9.771 | 104.8 |
| 9 | 10.321 | 99.2 |
| 10 | 9.559 | 107.1 |

</details>

### hugepages_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 0.541 | 0.031 |
| Throughput (MiB/s) | 1901.7 | 99.9 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 0.585 | 1750.8 |
| 2 | 0.519 | 1974.1 |
| 3 | 0.521 | 1965.5 |
| 4 | 0.512 | 1999.3 |
| 5 | 0.599 | 1709.5 |
| 6 | 0.517 | 1981.2 |
| 7 | 0.590 | 1736.3 |
| 8 | 0.525 | 1952.2 |
| 9 | 0.523 | 1956.2 |
| 10 | 0.514 | 1992.0 |

</details>

## m5.metal — AWS metal, Xeon 8259CL (Cascade Lake), 96 vCPU

### normal_no_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 1.473 | 0.032 |
| Throughput (MiB/s) | 695.3 | 14.6 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 1.516 | 675.5 |
| 2 | 1.451 | 705.8 |
| 3 | 1.506 | 679.9 |
| 4 | 1.438 | 711.9 |
| 5 | 1.510 | 678.1 |
| 6 | 1.443 | 709.5 |
| 7 | 1.435 | 713.6 |
| 8 | 1.494 | 685.2 |
| 9 | 1.496 | 684.5 |
| 10 | 1.443 | 709.4 |

</details>

### normal_no_uffd_dirty_tracking
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 1.483 | 0.033 |
| Throughput (MiB/s) | 691.0 | 15.2 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 1.520 | 673.8 |
| 2 | 1.454 | 704.2 |
| 3 | 1.509 | 678.6 |
| 4 | 1.436 | 713.1 |
| 5 | 1.457 | 703.0 |
| 6 | 1.513 | 676.6 |
| 7 | 1.509 | 678.4 |
| 8 | 1.454 | 704.4 |
| 9 | 1.526 | 671.2 |
| 10 | 1.448 | 707.0 |

</details>

### normal_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 6.780 | 0.723 |
| Throughput (MiB/s) | 152.9 | 16.1 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 6.054 | 169.1 |
| 2 | 7.733 | 132.4 |
| 3 | 7.131 | 143.6 |
| 4 | 6.108 | 167.6 |
| 5 | 5.979 | 171.3 |
| 6 | 7.582 | 135.1 |
| 7 | 6.048 | 169.3 |
| 8 | 7.365 | 139.0 |
| 9 | 7.831 | 130.8 |
| 10 | 5.971 | 171.5 |

</details>

### hugepages_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 0.632 | 0.079 |
| Throughput (MiB/s) | 1640.1 | 152.4 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 0.621 | 1648.3 |
| 2 | 0.587 | 1744.1 |
| 3 | 0.866 | 1183.0 |
| 4 | 0.617 | 1660.5 |
| 5 | 0.620 | 1652.0 |
| 6 | 0.587 | 1744.7 |
| 7 | 0.588 | 1740.7 |
| 8 | 0.596 | 1719.4 |
| 9 | 0.620 | 1650.5 |
| 10 | 0.618 | 1657.3 |

</details>

## c6i.metal — AWS metal, Xeon 8375C (Ice Lake), 128 vCPU

### normal_no_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 1.161 | 0.033 |
| Throughput (MiB/s) | 882.8 | 23.9 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 1.219 | 840.3 |
| 2 | 1.143 | 895.8 |
| 3 | 1.140 | 898.5 |
| 4 | 1.143 | 895.6 |
| 5 | 1.131 | 905.5 |
| 6 | 1.213 | 844.3 |
| 7 | 1.124 | 911.2 |
| 8 | 1.129 | 907.3 |
| 9 | 1.198 | 854.6 |
| 10 | 1.170 | 874.9 |

</details>

### normal_no_uffd_dirty_tracking
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 1.190 | 0.025 |
| Throughput (MiB/s) | 855.7 | 17.5 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 1.143 | 895.8 |
| 2 | 1.214 | 843.6 |
| 3 | 1.191 | 860.0 |
| 4 | 1.205 | 850.0 |
| 5 | 1.194 | 857.3 |
| 6 | 1.189 | 861.3 |
| 7 | 1.148 | 891.7 |
| 8 | 1.195 | 857.2 |
| 9 | 1.210 | 846.4 |
| 10 | 1.206 | 849.1 |

</details>

### normal_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 4.830 | 0.119 |
| Throughput (MiB/s) | 213.1 | 5.2 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 4.807 | 213.0 |
| 2 | 4.747 | 215.7 |
| 3 | 4.824 | 212.3 |
| 4 | 5.067 | 202.1 |
| 5 | 4.700 | 217.9 |
| 6 | 4.730 | 216.5 |
| 7 | 4.706 | 217.6 |
| 8 | 4.738 | 216.1 |
| 9 | 5.050 | 202.8 |
| 10 | 4.728 | 216.6 |

</details>

### hugepages_uffd
| Metric | Avg | Std Dev |
|--------|-----|---------|
| Duration (s) | 0.480 | 0.015 |
| Throughput (MiB/s) | 2138.0 | 63.8 |

<details><summary>Raw values</summary>

| Run | Duration (s) | Throughput (MiB/s) |
|-----|-------------|-------------------|
| 1 | 0.478 | 2143.6 |
| 2 | 0.462 | 2214.2 |
| 3 | 0.509 | 2010.0 |
| 4 | 0.472 | 2168.9 |
| 5 | 0.466 | 2197.1 |
| 6 | 0.474 | 2159.1 |
| 7 | 0.474 | 2158.3 |
| 8 | 0.474 | 2161.2 |
| 9 | 0.507 | 2018.5 |
| 10 | 0.477 | 2149.0 |

</details>
