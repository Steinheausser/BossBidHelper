# BossBidHelper
Vibe-coded repo to help with SMU Boss bidding in rust.
You will need to download / scrape the xlsx files for bids yourself. Place them (with the names unchanged, e.g. "2024-25_T2") into the data folder.
Then run using cargo run --release in the root folder.

A few issues: rust cannot read older xls files. Hence all files are converted into parquet. Furthermore, linear regression predictions for bids are wonky. Also included is a .py script that clicks through and scrapes BOSS tables if they are not released onto OASIS yet.

Feedback always welcome. 
Use at your own risk. This is AI slop after all.
